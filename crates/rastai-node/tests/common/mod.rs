//! What the node tests share: a fresh folder per test, data paths inside it, a machine built from a recorded
//! hardware fixture (`fixtures/hardware/<case>/`), stand-in tools, and a running node to talk to. Unix only, like the
//! tests that use it: the stand-in tools are shell scripts and the node serves a Unix socket.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rastai_core::paths::{DataPaths, node_socket_path};
use rastai_core::settings::{default_settings, node_settings, resolve_settings};
use rastai_node::error::NodeError;
use rastai_node::hardware::HardwareSources;
use rastai_node::node::{NodeMode, NodeOptions, run_node};
use rastai_proto::node::v1::node_service_client::NodeServiceClient;
use rastai_transport::grpc_channel::connect_grpc_channel;
use tonic::transport::Channel;

/// The recorded machines, relative to this crate's folder.
pub const FIXTURES_FOLDER: &str = "../../fixtures/hardware";
/// A stand-in tool that never answers.
pub const HUNG_TOOL_SCRIPT: &str = "#!/bin/sh\nexec sleep 600\n";
/// How long a test waits for a started node to answer on node.sock.
const NODE_START_WAIT: Duration = Duration::from_secs(20);
const NODE_START_POLL: Duration = Duration::from_millis(20);
const EXECUTABLE_MODE: u32 = 0o755;
/// Where the stand-in nvidia-smi's telemetry stream writes its process id, in the machine's tools folder.
const STREAM_PID_FILE: &str = "nvidia-smi-stream.pid";

/// Numbers the folders of one test binary, so tests running at the same time never share one.
static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

/// How the stand-in nvidia-smi behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaSmi {
    /// Prints the fixture's recorded outputs: the GPU query, the telemetry stream and the process query.
    Recorded,
    /// Never answers.
    Hung,
}

/// One test's folder, removed when the test ends, also when an assertion fails.
pub struct TestFolder {
    pub path: PathBuf,
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A fresh, empty folder for one test. The name stays short so node.sock inside it fits a Unix socket address.
#[allow(clippy::unwrap_used)]
pub fn new_test_folder(prefix: &str) -> TestFolder {
    let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{number}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TestFolder { path }
}

/// The file where the stand-in nvidia-smi of the machine in `folder` writes its telemetry stream's process id.
pub fn stream_pid_path(folder: &Path) -> PathBuf {
    folder.join("bin").join(STREAM_PID_FILE)
}

/// The five data roots, each a folder inside the test folder.
pub fn test_paths(folder: &Path) -> DataPaths {
    DataPaths {
        config: folder.join("config"),
        data: folder.join("data"),
        cache: folder.join("cache"),
        state: folder.join("state"),
        runtime: folder.join("run"),
    }
}

pub fn fixture_folder(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_FOLDER).join(case)
}

/// A machine laid out like /sys and /proc from a recorded fixture, with the stand-in tools it had. The fixture
/// stores PCI addresses with `-` for `:`, which Windows can't check out; they get their `:` back here.
#[allow(clippy::unwrap_used)]
pub fn machine_from_fixture(
    case: &str,
    folder: &Path,
    nvidia_smi: NvidiaSmi,
    tool_timeout: Duration,
) -> HardwareSources {
    let fixture = fixture_folder(case);
    let sys_root = folder.join("sys");
    let proc_root = folder.join("proc");
    copy_folder(&fixture.join("proc"), &proc_root);
    for address in folder_names(&fixture.join("pci")) {
        let target = sys_root.join("bus/pci/devices").join(address.replace('-', ":"));
        copy_folder(&fixture.join("pci").join(&address), &target);
    }
    for node in folder_names(&fixture.join("kfd")) {
        let target = sys_root.join("class/kfd/kfd/topology/nodes").join(&node);
        copy_folder(&fixture.join("kfd").join(&node), &target);
    }
    fs::create_dir_all(&sys_root).unwrap();
    let tools = folder.join("bin");
    fs::create_dir_all(&tools).unwrap();
    let recorded_gpus = fixture.join("tools/nvidia-smi-gpus.csv");
    let nvidia_smi_path = tools.join("nvidia-smi");
    match nvidia_smi {
        NvidiaSmi::Recorded if recorded_gpus.exists() => {
            let script = recorded_nvidia_smi(&fixture, &stream_pid_path(folder));
            write_tool(&nvidia_smi_path, &script);
        }
        NvidiaSmi::Recorded => {}
        NvidiaSmi::Hung => write_tool(&nvidia_smi_path, HUNG_TOOL_SCRIPT),
    }
    HardwareSources {
        sys_root,
        proc_root,
        nvidia_smi: nvidia_smi_path,
        // No fixture box had ROCm installed, so rocm-smi is never there.
        rocm_smi: tools.join("rocm-smi"),
        tool_timeout,
    }
}

/// Node options for a test: an embedded node in `folder`, reading `sources`.
pub fn test_node_options(folder: &Path, sources: HardwareSources) -> NodeOptions {
    NodeOptions {
        mode: NodeMode::Embedded,
        paths: test_paths(folder),
        settings: node_settings(&resolve_settings(&[default_settings()])),
        hardware_sources: sources,
    }
}

/// Starts a node on its own task and returns once node.sock answers, with a client for it.
#[allow(clippy::unwrap_used)]
pub async fn start_node(
    options: NodeOptions,
) -> (
    tokio::task::JoinHandle<Result<(), NodeError>>,
    NodeServiceClient<Channel>,
) {
    let socket_path = node_socket_path(&options.paths);
    let node = tokio::spawn(run_node(options));
    let started = Instant::now();
    loop {
        if let Ok(channel) = connect_grpc_channel(&socket_path).await {
            return (node, NodeServiceClient::new(channel));
        }
        assert!(!node.is_finished(), "the node stopped: {:?}", node.await);
        assert!(
            started.elapsed() < NODE_START_WAIT,
            "node.sock didn't answer in {NODE_START_WAIT:?}"
        );
        tokio::time::sleep(NODE_START_POLL).await;
    }
}

/// A stand-in nvidia-smi that answers each query with the fixture's recording. The telemetry stream stays open
/// after its lines, as the real `-l 5` does; `exec` makes sleep the process the node kills, under the process id
/// written to `pid_path` first.
fn recorded_nvidia_smi(fixture: &Path, pid_path: &Path) -> String {
    let tools = fixture.join("tools");
    let stream = tools.join("nvidia-smi-telemetry-stream.csv");
    let processes = tools.join("nvidia-smi-compute-apps.csv");
    format!(
        "#!/bin/sh\ncase \"$*\" in\n  *pci.bus_id*) cat '{}' ;;\n  *utilization.gpu*) echo $$ > '{}'; cat '{}' 2>/dev/null; exec sleep 600 ;;\n  *compute-apps*) cat '{}' 2>/dev/null ;;\nesac\n",
        tools.join("nvidia-smi-gpus.csv").display(),
        pid_path.display(),
        stream.display(),
        processes.display()
    )
}

#[allow(clippy::unwrap_used)]
pub fn write_tool(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
}

#[allow(clippy::unwrap_used)]
fn copy_folder(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}

fn folder_names(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect()
}
