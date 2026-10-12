//! What the node tests share: a fresh folder per test, data paths inside it, a machine built from a recorded
//! hardware fixture (`fixtures/hardware/<case>/`), stand-in tools, a running node to talk to, and the daemon's side of
//! the node's link: a pinned daemon's handshake and a stand-in `NodeHubService` that keeps the heartbeats it gets.
//! Unix only, like the tests that use it: the stand-in tools are shell scripts and the node serves a Unix socket.
#![allow(dead_code)]

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rastai_core::paths::{DataPaths, node_key_path, node_socket_path, paired_daemons_path};
use rastai_core::random::random_bytes;
use rastai_core::settings::{default_settings, node_settings, resolve_settings};
use rastai_core::time::now_unix_ms;
use rastai_core::versions::{APP_VERSION, NODE_PROTOCOL_VERSION};
use rastai_node::error::NodeError;
use rastai_node::handshake::{daemon_challenge_bytes, pin_daemon_key};
use rastai_node::hardware::HardwareSources;
use rastai_node::identity::{daemon_id_from_public_key, load_or_create_identity, public_key_bytes, sign_challenge};
use rastai_node::jobs::supervisor::default_job_options;
use rastai_node::node::{NodeMode, NodeOptions, run_node};
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use rastai_proto::node::v1::node_hub_service_server::{NodeHubService, NodeHubServiceServer};
use rastai_proto::node::v1::node_service_client::NodeServiceClient;
use rastai_proto::node::v1::{HandshakeRequest, HeartbeatRequest, HeartbeatResponse};
use rastai_transport::grpc_channel::{connect_grpc_channel, serve_grpc_on_socket};
use rastai_transport::local_socket::bind_local_socket;
use tonic::transport::{Channel, Server};
use tonic::{Request, Response, Status};

/// The recorded machines, relative to this crate's folder.
pub const FIXTURES_FOLDER: &str = "../../fixtures/hardware";
/// A stand-in tool that never answers.
pub const HUNG_TOOL_SCRIPT: &str = "#!/bin/sh\nexec sleep 600\n";
/// How long a test waits for a started node to answer on node.sock.
const NODE_START_WAIT: Duration = Duration::from_secs(20);
const NODE_START_POLL: Duration = Duration::from_millis(20);
const EXECUTABLE_MODE: u32 = 0o755;
/// `write_tool` writes a script here first, beside its final path.
const STAGING_SUFFIX: &str = "staging";
/// Where the stand-in nvidia-smi's telemetry stream writes its process id, in the machine's tools folder.
const STREAM_PID_FILE: &str = "nvidia-smi-stream.pid";
/// The test daemon's key, in the test folder.
const DAEMON_KEY_FILE: &str = "daemon.key";
/// The daemon's nonce: 32 random bytes, as rastai-server sends.
const NONCE_BYTES: usize = 32;
/// How long a test waits for a heartbeat that should come.
const HEARTBEAT_WAIT: Duration = Duration::from_secs(10);

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

/// Node options for a test: an embedded node in `folder`, reading `sources`, with the product's job times and no
/// environments (the job tests add theirs).
pub fn test_node_options(folder: &Path, sources: HardwareSources) -> NodeOptions {
    NodeOptions {
        mode: NodeMode::Embedded,
        paths: test_paths(folder),
        settings: node_settings(&resolve_settings(&[default_settings()])),
        hardware_sources: sources,
        jobs: default_job_options(Vec::new()),
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

/// Writes an executable stand-in tool. The tests of one binary run on parallel threads, and a child another thread
/// forks inherits every descriptor this process has open until its own exec. Had this process written the tool
/// itself, such a child could still hold it open for writing when the tool runs, and the kernel refuses to run a file
/// open for writing (ETXTBSY, issue #209). Renaming a finished file into place doesn't help: the child holds the
/// same file. So the script is written to a staging file, and `cp`, a process of its own whose descriptors nobody
/// else inherits, writes the tool.
#[allow(clippy::unwrap_used)]
pub fn write_tool(path: &Path, script: &str) {
    let staging = path.with_extension(STAGING_SUFFIX);
    fs::write(&staging, script).unwrap();
    let copied = std::process::Command::new("cp")
        .arg(&staging)
        .arg(path)
        .status()
        .unwrap();
    assert!(
        copied.success(),
        "cp {} {}: {copied}",
        staging.display(),
        path.display()
    );
    fs::remove_file(&staging).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
}

/// Waits until nobody listens on `socket_path`. A node stopped inside the test process lets go of its sockets from
/// detached tasks a moment after its own task has ended (issue #233), and a node started again at once in the same
/// folder would find them still answering and refuse to start.
#[allow(clippy::unwrap_used)]
pub async fn wait_until_socket_released(socket_path: &Path) {
    let started = Instant::now();
    loop {
        match UnixStream::connect(socket_path) {
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => return,
            Err(error) if error.kind() == ErrorKind::NotFound => return,
            _ => {}
        }
        assert!(
            started.elapsed() < NODE_START_WAIT,
            "{} still answers after {NODE_START_WAIT:?}",
            socket_path.display()
        );
        tokio::time::sleep(NODE_START_POLL).await;
    }
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

/// The handshake of a daemon whose key is pinned for the node in `folder`, naming `hub_socket_path` as its hub, signed
/// as rastai-server signs it.
#[allow(clippy::unwrap_used)]
pub fn paired_daemon_handshake(folder: &Path, hub_socket_path: &Path) -> HandshakeRequest {
    let paths = test_paths(folder);
    let node = load_or_create_identity(&node_key_path(&paths)).unwrap();
    let node_id = rastai_core::ids::node_id_from_public_key(&public_key_bytes(&node));
    let daemon = load_or_create_identity(&folder.join(DAEMON_KEY_FILE)).unwrap();
    let daemon_id = daemon_id_from_public_key(&public_key_bytes(&daemon));
    pin_daemon_key(&paired_daemons_path(&paths), &daemon_id).unwrap();
    let hub_socket_path = hub_socket_path.to_string_lossy().into_owned();
    let nonce = random_bytes(NONCE_BYTES).unwrap();
    let sent_unix_ms = now_unix_ms();
    let signed = daemon_challenge_bytes(&node_id, &nonce, sent_unix_ms, &hub_socket_path);
    HandshakeRequest {
        protocol_version: NODE_PROTOCOL_VERSION,
        capabilities: Vec::new(),
        app_version: APP_VERSION.to_string(),
        daemon_id,
        daemon_nonce: nonce,
        sent_unix_ms,
        daemon_signature: sign_challenge(&daemon, &signed),
        hub_socket_path,
    }
}

/// The daemon's `NodeHubService` for a test: it keeps every heartbeat it gets, with when it got it. Serving stops
/// when it is dropped.
pub struct FakeHub {
    heartbeats: Arc<Mutex<Vec<(Instant, HeartbeatRequest)>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for FakeHub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct FakeHubService {
    heartbeats: Arc<Mutex<Vec<(Instant, HeartbeatRequest)>>>,
}

#[tonic::async_trait]
impl NodeHubService for FakeHubService {
    async fn heartbeat(&self, request: Request<HeartbeatRequest>) -> Result<Response<HeartbeatResponse>, Status> {
        let mut heartbeats = self
            .heartbeats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        heartbeats.push((Instant::now(), request.into_inner()));
        Ok(Response::new(HeartbeatResponse {}))
    }

    async fn execute_command(&self, _request: Request<ExecuteRequest>) -> Result<Response<ExecuteResponse>, Status> {
        Err(Status::unimplemented("the test hub runs no commands"))
    }
}

/// Serves a `FakeHub` on `socket_path`.
#[allow(clippy::unwrap_used)]
pub fn start_fake_hub(socket_path: &Path) -> FakeHub {
    let heartbeats = Arc::new(Mutex::new(Vec::new()));
    let service = FakeHubService {
        heartbeats: Arc::clone(&heartbeats),
    };
    let listener = bind_local_socket(socket_path).unwrap();
    let router = Server::builder().add_service(NodeHubServiceServer::new(service));
    let handle = tokio::spawn(async move {
        let _ = serve_grpc_on_socket(listener, router).await;
    });
    FakeHub { heartbeats, handle }
}

impl FakeHub {
    /// Every heartbeat so far, oldest first, with when it arrived.
    pub fn heartbeats(&self) -> Vec<(Instant, HeartbeatRequest)> {
        self.heartbeats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Waits for the first heartbeat after the first `skip` that `wanted` accepts, and returns it with when it came.
    pub async fn wait_for_heartbeat(
        &self,
        skip: usize,
        wanted: impl Fn(&HeartbeatRequest) -> bool,
    ) -> (Instant, HeartbeatRequest) {
        let started = Instant::now();
        loop {
            let heartbeats = self.heartbeats();
            let found = heartbeats.into_iter().skip(skip).find(|(_, beat)| wanted(beat));
            if let Some(found) = found {
                return found;
            }
            assert!(
                started.elapsed() < HEARTBEAT_WAIT,
                "no such heartbeat in {HEARTBEAT_WAIT:?}: {:?}",
                self.heartbeats()
            );
            tokio::time::sleep(NODE_START_POLL).await;
        }
    }
}
