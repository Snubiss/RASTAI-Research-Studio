//! How the node launches a Python worker (layout: rust-node-gateway-proto.md section 7.4, "How spawn.rs launches a
//! worker"): `setsid <env>/bin/python -m rastai_worker run --job-file <run folder>/job.json`, with stdin from
//! `/dev/null`, stdout and stderr appended to the run folder's log files, and a cleared environment.
//!
//! `setsid` (util-linux) gives the worker a new session and process group under its own pid with no `unsafe` code,
//! so the node can signal the whole group (signals.rs) and a dead node sends the job no SIGHUP. Where `setsid` is
//! missing, std's `process_group(0)` is the fallback (a new group, but no new session).
//!
//! The environment starts empty. Only `PATH`, `HOME`, `LANG`, `TZ` and `XDG_*` are copied from the node; the job's
//! token, run id, GPUs and caches are added; downloads and installs are turned off. `HSA_OVERRIDE_GFX_VERSION` is
//! never passed on: forcing ROCm on an unsupported chip gave silently wrong matmuls (spike 05). A debug build also
//! passes on the kill -9 tests' `RASTAI_TEST_PAUSE_AT` (pause_points.rs); a release build never does.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

#[cfg(unix)]
use rastai_transport::local_socket::SOCKET_DIRECTORY_MODE;
use tokio::process::{Child, Command};

use crate::jobs::log_lines::{STDERR_LOG_FILE, STDOUT_LOG_FILE};
use crate::jobs::signals::{ProcessIdentity, read_process_identity};
use crate::pause_points::worker_test_pause_variable;

/// The tool that starts the worker in a new session and process group.
const SETSID_TOOL: &str = "setsid";
/// The interpreter inside an environment folder.
const PYTHON_IN_ENVIRONMENT: &str = "bin/python";
/// What comes between the interpreter and the job file (contracts 3.1).
const WORKER_ARGUMENTS: &[&str] = &["-m", "rastai_worker", "run", "--job-file"];
/// The logs hold the job's output, which may quote its data, so only the user may read them.
#[cfg(unix)]
const LOG_FILE_MODE: u32 = 0o600;
/// How long the node waits for `setsid` to make the new group before it gives up on the start. setsid does it in
/// its first microseconds; the bound only matters on a machine that is badly overloaded.
const PROCESS_GROUP_WAIT: Duration = Duration::from_secs(5);
const PROCESS_GROUP_POLL: Duration = Duration::from_millis(2);
/// Copied from the node's own environment, by name and by prefix.
const COPIED_VARIABLES: &[&str] = &["PATH", "HOME", "LANG", "TZ"];
const COPIED_VARIABLE_PREFIX: &str = "XDG_";
const PATH_VARIABLE: &str = "PATH";
/// Never passed to a job (spike 05); its removal is logged as `RRS-H4002`.
pub const GFX_OVERRIDE_VARIABLE: &str = "HSA_OVERRIDE_GFX_VERSION";
/// Folders under `<cache>/torch/` for the compile caches (contracts 8.1: compile caches, safe to delete).
const INDUCTOR_CACHE_FOLDER: &str = "inductor";
const TRITON_CACHE_FOLDER: &str = "triton";
/// CUDA's default order is fastest first; the bus order matches nvidia-smi's numbering (platform flow 5.1).
const CUDA_DEVICE_ORDER: &str = "PCI_BUS_ID";
/// "1" turns a switch on.
const ON: &str = "1";
/// The variable that carries the job token to the worker (contracts 3.2).
const JOB_TOKEN_VARIABLE: &str = "RASTAI_JOB_TOKEN";
/// The test-only CPU thread cap, named in the test environment (`just test-crash`, `just test-daemon-jobs`).
const TEST_WORKER_THREADS_VARIABLE: &str = "RASTAI_TEST_WORKER_THREADS";
/// What torch reads for its CPU threads; the cap is handed to the worker under this name.
const THREAD_COUNT_VARIABLE: &str = "OMP_NUM_THREADS";
/// What a `{:?}` prints in place of the job token. The token lets whoever holds it act as the job's worker, and a
/// struct printed into a log or a test failure must not hand it out, so every struct that holds it has a
/// hand-written Debug that prints this instead.
pub(crate) const REDACTED_TOKEN: &str = "<redacted>";

/// What the job's environment variables are made from, besides the node's own environment. Debug is hand-written:
/// it never prints the token.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkerVariableInputs {
    /// The 64 hex characters of the job token (`RASTAI_JOB_TOKEN`).
    pub job_token: String,
    pub run_id: String,
    /// The job's GPUs by physical index; the job sees them in ascending order and no others.
    pub gpu_indices: Vec<u32>,
    /// The node computes on AMD GPUs, so the GPUs are named in `HIP_VISIBLE_DEVICES`.
    pub amd_gpus: bool,
    /// `<cache>/torch/`.
    pub torch_cache_folder: PathBuf,
    /// `<data>/plugin-data/<plugin_id>/` for a job with a `plugin_id`.
    pub plugin_data_folder: Option<PathBuf>,
}

/// The job's environment, and whether the node had `HSA_OVERRIDE_GFX_VERSION` set (and so left it out). Debug is
/// hand-written: it never prints the token's value.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkerVariables {
    pub variables: Vec<(String, String)>,
    pub removed_gfx_override: bool,
}

/// Everything one launch needs. Debug is hand-written: it never prints the token's value.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkerLaunch {
    /// The environment folder; its interpreter is `bin/python`.
    pub environment_folder: PathBuf,
    pub job_file: PathBuf,
    pub run_folder: PathBuf,
    pub variables: Vec<(String, String)>,
}

/// A started worker: the child to wait on, and who it is, for the pid-reuse check.
#[derive(Debug)]
pub struct SpawnedWorker {
    pub child: Child,
    pub identity: ProcessIdentity,
    /// `stdout.log`'s and `stderr.log`'s lengths before this worker wrote to them: where its own output starts
    /// (log_lines.rs).
    pub stdout_start_offset: u64,
    pub stderr_start_offset: u64,
}

impl std::fmt::Debug for WorkerVariableInputs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerVariableInputs")
            .field("job_token", &REDACTED_TOKEN)
            .field("run_id", &self.run_id)
            .field("gpu_indices", &self.gpu_indices)
            .field("amd_gpus", &self.amd_gpus)
            .field("torch_cache_folder", &self.torch_cache_folder)
            .field("plugin_data_folder", &self.plugin_data_folder)
            .finish()
    }
}

impl std::fmt::Debug for WorkerVariables {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerVariables")
            .field("variables", &redacted_variables(&self.variables))
            .field("removed_gfx_override", &self.removed_gfx_override)
            .finish()
    }
}

impl std::fmt::Debug for WorkerLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerLaunch")
            .field("environment_folder", &self.environment_folder)
            .field("job_file", &self.job_file)
            .field("run_folder", &self.run_folder)
            .field("variables", &redacted_variables(&self.variables))
            .finish()
    }
}

/// The variables as Debug shows them: the job token's value is replaced, every other one is kept.
fn redacted_variables(variables: &[(String, String)]) -> Vec<(&str, &str)> {
    let mut shown = Vec::with_capacity(variables.len());
    for (name, value) in variables {
        if name == JOB_TOKEN_VARIABLE {
            shown.push((name.as_str(), REDACTED_TOKEN));
        } else {
            shown.push((name.as_str(), value.as_str()));
        }
    }
    shown
}

/// The node's own environment variables. One that isn't UTF-8 can't be one the job is given, so it is skipped.
pub fn node_variables() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The CPU thread cap a test names for its workers, from the node's own environment. Several test workers on one
/// machine would each start a thread per core, so the crash rows and the daemon-job tests cap them. Only a test build
/// passes it on: a release node never does, so a shipped worker keeps torch's own default.
fn worker_test_threads(node_variables: &[(String, String)]) -> Option<String> {
    worker_test_threads_for(cfg!(debug_assertions), node_variables)
}

/// The decision of `worker_test_threads`, with the build given: the tests check both answers in every build.
fn worker_test_threads_for(build_is_debug: bool, node_variables: &[(String, String)]) -> Option<String> {
    if !build_is_debug {
        return None;
    }
    for (name, value) in node_variables {
        if name == TEST_WORKER_THREADS_VARIABLE && !value.is_empty() {
            return Some(value.clone());
        }
    }
    None
}

/// The job's environment: the copied variables, then the job's own (layout 7.4's table).
pub fn worker_variables(node_variables: &[(String, String)], inputs: &WorkerVariableInputs) -> WorkerVariables {
    let mut variables: Vec<(String, String)> = node_variables
        .iter()
        .filter(|(name, _)| is_copied_variable(name))
        .cloned()
        .collect();
    push_variable(&mut variables, JOB_TOKEN_VARIABLE, inputs.job_token.clone());
    push_variable(&mut variables, "RASTAI_RUN_ID", inputs.run_id.clone());
    push_variable(&mut variables, "CUDA_DEVICE_ORDER", CUDA_DEVICE_ORDER.to_string());
    let visible_name = visible_devices_variable(inputs.amd_gpus);
    push_variable(&mut variables, visible_name, visible_devices(&inputs.gpu_indices));
    push_variable(&mut variables, "HF_HUB_OFFLINE", ON.to_string());
    push_variable(&mut variables, "UV_OFFLINE", ON.to_string());
    push_variable(&mut variables, "PIP_NO_INDEX", ON.to_string());
    let inductor = inputs.torch_cache_folder.join(INDUCTOR_CACHE_FOLDER);
    let triton = inputs.torch_cache_folder.join(TRITON_CACHE_FOLDER);
    push_variable(&mut variables, "TORCHINDUCTOR_CACHE_DIR", path_text(&inductor));
    push_variable(&mut variables, "TRITON_CACHE_DIR", path_text(&triton));
    if let Some(folder) = &inputs.plugin_data_folder {
        push_variable(&mut variables, "RASTAI_PLUGIN_DATA_DIR", path_text(folder));
    }
    // The kill -9 tests' pause point, in debug builds only (pause_points.rs): a worker point stops the worker.
    if let Some((name, point)) = worker_test_pause_variable(node_variables) {
        push_variable(&mut variables, &name, point);
    }
    if let Some(threads) = worker_test_threads(node_variables) {
        push_variable(&mut variables, THREAD_COUNT_VARIABLE, threads);
    }
    let removed_gfx_override = node_variables.iter().any(|(name, _)| name == GFX_OVERRIDE_VARIABLE);
    WorkerVariables {
        variables,
        removed_gfx_override,
    }
}

/// The interpreter of an environment folder.
pub fn environment_python(environment_folder: &Path) -> PathBuf {
    environment_folder.join(PYTHON_IN_ENVIRONMENT)
}

/// Creates the folder of a job socket (`<runtime>/jobs/`), mode 0700, when it is missing. An existing folder is left
/// as it is: the gRPC channel refuses one another user owns or can enter (rastai-transport).
pub fn prepare_job_socket_folder(job_socket_path: &Path) -> Result<(), String> {
    let Some(folder) = job_socket_path.parent() else {
        return Ok(());
    };
    if folder.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    set_folder_mode(&mut builder);
    builder
        .create(folder)
        .map_err(|error| format!("the job socket folder {} can't be created: {error}", folder.display()))
}

/// Starts the worker and returns once it leads its own process group. The error says what failed; nothing is left
/// running after one.
pub async fn spawn_worker(launch: &WorkerLaunch) -> Result<SpawnedWorker, String> {
    let python = environment_python(&launch.environment_folder);
    if !python.is_file() {
        return Err(format!("the environment's interpreter {} is missing", python.display()));
    }
    let stdout = open_log_file(&launch.run_folder.join(STDOUT_LOG_FILE))?;
    let stderr = open_log_file(&launch.run_folder.join(STDERR_LOG_FILE))?;
    let stdout_start_offset = log_file_length(&stdout);
    let stderr_start_offset = log_file_length(&stderr);
    let mut command = worker_command(&python, launch);
    command
        .env_clear()
        .envs(
            launch
                .variables
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let mut child = command
        .spawn()
        .map_err(|error| format!("{} can't be started: {error}", python.display()))?;
    let Some(pid) = child.id() else {
        return Err("the worker ended before its process id could be read".to_string());
    };
    match wait_for_new_process_group(pid).await {
        Ok(identity) => Ok(SpawnedWorker {
            child,
            identity,
            stdout_start_offset,
            stderr_start_offset,
        }),
        Err(reason) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(reason)
        }
    }
}

/// `setsid <python> -m rastai_worker run --job-file <path>`, or the same without `setsid` in a new process group.
fn worker_command(python: &Path, launch: &WorkerLaunch) -> Command {
    let node_path = std::env::var_os(PATH_VARIABLE).unwrap_or_default();
    let mut command = match find_on_path(SETSID_TOOL, &node_path) {
        Some(setsid) => {
            let mut command = Command::new(setsid);
            command.arg(python);
            command
        }
        None => new_group_command(python),
    };
    command.args(WORKER_ARGUMENTS).arg(&launch.job_file);
    command
}

#[cfg(unix)]
fn new_group_command(python: &Path) -> Command {
    let mut command = Command::new(python);
    command.process_group(0);
    command
}

/// Process groups are Unix; Windows jobs come with the Windows build (tier 2).
#[cfg(not(unix))]
fn new_group_command(python: &Path) -> Command {
    Command::new(python)
}

/// The first `name` in the folders of `path_value`.
fn find_on_path(name: &str, path_value: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_value)
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

/// Polls until the process leads its own group (setsid has run), then reads its identity. A process that already
/// exited stays readable as a zombie until the node waits on it, so it still gets an identity.
async fn wait_for_new_process_group(pid: u32) -> Result<ProcessIdentity, String> {
    let started = Instant::now();
    loop {
        match read_process_identity(pid) {
            Some(identity) if identity.process_group_id == pid => return Ok(identity),
            _ => {}
        }
        if started.elapsed() > PROCESS_GROUP_WAIT {
            return Err(format!(
                "process {pid} didn't get a process group of its own within {} seconds",
                PROCESS_GROUP_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(PROCESS_GROUP_POLL).await;
    }
}

/// Opens a log for appending, mode 0600 when new: a relaunch of the run adds to what the earlier job wrote.
fn open_log_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    set_file_mode(&mut options);
    options
        .open(path)
        .map_err(|error| format!("{} can't be opened: {error}", path.display()))
}

/// A log's length, read before the worker gets it. One whose length can't be read counts from 0: the tail may then
/// quote an earlier job, which is better than quoting nothing.
fn log_file_length(file: &File) -> u64 {
    file.metadata().map(|metadata| metadata.len()).unwrap_or(0)
}

#[cfg(unix)]
fn set_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(LOG_FILE_MODE);
}

#[cfg(not(unix))]
fn set_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_folder_mode(builder: &mut std::fs::DirBuilder) {
    use std::os::unix::fs::DirBuilderExt;

    builder.mode(SOCKET_DIRECTORY_MODE);
}

#[cfg(not(unix))]
fn set_folder_mode(_builder: &mut std::fs::DirBuilder) {}

fn push_variable(variables: &mut Vec<(String, String)>, name: &str, value: String) {
    variables.push((name.to_string(), value));
}

fn is_copied_variable(name: &str) -> bool {
    COPIED_VARIABLES.contains(&name) || name.starts_with(COPIED_VARIABLE_PREFIX)
}

fn visible_devices_variable(amd_gpus: bool) -> &'static str {
    if amd_gpus {
        "HIP_VISIBLE_DEVICES"
    } else {
        "CUDA_VISIBLE_DEVICES"
    }
}

/// `0,2` for GPUs 2 and 0. An empty value hides every GPU, which is right for a job that leased none.
fn visible_devices(gpu_indices: &[u32]) -> String {
    let mut sorted = gpu_indices.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let names: Vec<String> = sorted.iter().map(u32::to_string).collect();
    names.join(",")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pause_points::worker_test_pause_variable_for;

    fn inputs() -> WorkerVariableInputs {
        WorkerVariableInputs {
            job_token: "ab".repeat(32),
            run_id: "run_x".to_string(),
            gpu_indices: vec![2, 0],
            amd_gpus: false,
            torch_cache_folder: PathBuf::from("/c/torch"),
            plugin_data_folder: None,
        }
    }

    fn node() -> Vec<(String, String)> {
        let pairs = [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/u"),
            ("LANG", "C.UTF-8"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("HSA_OVERRIDE_GFX_VERSION", "10.3.0"),
            ("HF_TOKEN", "hf_secret"),
            ("CUDA_VISIBLE_DEVICES", "0,1,2"),
            ("PYTHONPATH", "/somewhere"),
        ];
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn value_of<'a>(variables: &'a WorkerVariables, name: &str) -> Option<&'a str> {
        let found = variables.variables.iter().find(|(key, _)| key == name);
        found.map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_environment_holds_only_what_the_job_needs() {
        let result = worker_variables(&node(), &inputs());
        assert_eq!(value_of(&result, "PATH"), Some("/usr/bin"));
        assert_eq!(value_of(&result, "XDG_RUNTIME_DIR"), Some("/run/user/1000"));
        assert_eq!(value_of(&result, "RASTAI_RUN_ID"), Some("run_x"));
        assert_eq!(value_of(&result, "CUDA_DEVICE_ORDER"), Some("PCI_BUS_ID"));
        assert_eq!(value_of(&result, "CUDA_VISIBLE_DEVICES"), Some("0,2"));
        assert_eq!(value_of(&result, "HF_HUB_OFFLINE"), Some("1"));
        assert_eq!(value_of(&result, "UV_OFFLINE"), Some("1"));
        assert_eq!(value_of(&result, "PIP_NO_INDEX"), Some("1"));
        assert_eq!(value_of(&result, "TORCHINDUCTOR_CACHE_DIR"), Some("/c/torch/inductor"));
        assert_eq!(value_of(&result, "TRITON_CACHE_DIR"), Some("/c/torch/triton"));
        for removed in [
            "HSA_OVERRIDE_GFX_VERSION",
            "HF_TOKEN",
            "PYTHONPATH",
            "RASTAI_PLUGIN_DATA_DIR",
        ] {
            assert_eq!(value_of(&result, removed), None, "{removed}");
        }
        assert!(result.removed_gfx_override);
        let names: Vec<&str> = result.variables.iter().map(|(name, _)| name.as_str()).collect();
        let visible = names.iter().filter(|name| **name == "CUDA_VISIBLE_DEVICES").count();
        assert_eq!(visible, 1, "the node's own GPU list is never passed on");
    }

    #[test]
    fn amd_gpus_a_plugin_folder_and_no_override() {
        let amd = WorkerVariableInputs {
            amd_gpus: true,
            gpu_indices: Vec::new(),
            plugin_data_folder: Some(PathBuf::from("/d/plugin-data/evaluation")),
            ..inputs()
        };
        let node_without_override: Vec<(String, String)> = node()
            .into_iter()
            .filter(|(name, _)| name != GFX_OVERRIDE_VARIABLE)
            .collect();
        let result = worker_variables(&node_without_override, &amd);
        assert_eq!(value_of(&result, "HIP_VISIBLE_DEVICES"), Some(""));
        assert_eq!(value_of(&result, "CUDA_VISIBLE_DEVICES"), None);
        assert_eq!(
            value_of(&result, "RASTAI_PLUGIN_DATA_DIR"),
            Some("/d/plugin-data/evaluation")
        );
        assert!(!result.removed_gfx_override);
    }

    // The kill -9 tests name a worker point in the daemon's environment; the cleared environment must still carry
    // it to the worker (test builds are debug builds), and without it the worker gets no such variable.
    // Both answers of each build decision, in every build (a release build's answer is the shipped one).
    #[test]
    fn the_build_decides_what_the_test_variables_reach_the_worker() {
        let mut with_test_variables = node();
        with_test_variables.push(("RASTAI_TEST_PAUSE_AT".to_string(), "worker.point".to_string()));
        with_test_variables.push(("RASTAI_TEST_WORKER_THREADS".to_string(), "2".to_string()));
        assert_eq!(
            worker_test_threads_for(true, &with_test_variables),
            Some("2".to_string())
        );
        assert_eq!(worker_test_threads_for(false, &with_test_variables), None);
        assert!(worker_test_pause_variable_for(true, &with_test_variables).is_some());
        assert_eq!(worker_test_pause_variable_for(false, &with_test_variables), None);
    }

    // Test builds are debug builds: the point the kill -9 tests name reaches the worker.
    #[cfg(debug_assertions)]
    #[test]
    fn the_test_pause_point_reaches_the_worker() {
        assert_eq!(
            value_of(&worker_variables(&node(), &inputs()), "RASTAI_TEST_PAUSE_AT"),
            None
        );
        let mut with_point = node();
        with_point.push((
            "RASTAI_TEST_PAUSE_AT".to_string(),
            "worker.event_log.mid_metrics_frame".to_string(),
        ));
        let result = worker_variables(&with_point, &inputs());
        assert_eq!(
            value_of(&result, "RASTAI_TEST_PAUSE_AT"),
            Some("worker.event_log.mid_metrics_frame")
        );
    }

    // The release twin: a shipped node never passes the point on, so the worker never sees the variable.
    #[cfg(not(debug_assertions))]
    #[test]
    fn the_test_pause_point_never_reaches_the_worker_in_a_release_build() {
        let mut with_point = node();
        with_point.push((
            "RASTAI_TEST_PAUSE_AT".to_string(),
            "worker.event_log.mid_metrics_frame".to_string(),
        ));
        let result = worker_variables(&with_point, &inputs());
        assert_eq!(value_of(&result, "RASTAI_TEST_PAUSE_AT"), None);
    }

    // Several test workers train at once; the cap the test names must reach each worker as its thread count, and a
    // worker started without a test environment gets none, so a shipped node is unchanged.
    #[cfg(debug_assertions)]
    #[test]
    fn the_test_thread_cap_reaches_the_worker_as_omp_num_threads() {
        assert_eq!(value_of(&worker_variables(&node(), &inputs()), "OMP_NUM_THREADS"), None);
        let mut capped = node();
        capped.push(("RASTAI_TEST_WORKER_THREADS".to_string(), "2".to_string()));
        let result = worker_variables(&capped, &inputs());
        assert_eq!(value_of(&result, "OMP_NUM_THREADS"), Some("2"));
        assert_eq!(value_of(&result, "RASTAI_TEST_WORKER_THREADS"), None);
    }

    // The release twin: a shipped node passes no thread cap, so the worker keeps torch's own default.
    #[cfg(not(debug_assertions))]
    #[test]
    fn the_test_thread_cap_never_reaches_the_worker_in_a_release_build() {
        let mut capped = node();
        capped.push(("RASTAI_TEST_WORKER_THREADS".to_string(), "2".to_string()));
        let result = worker_variables(&capped, &inputs());
        assert_eq!(value_of(&result, "OMP_NUM_THREADS"), None);
    }

    // A token printed with {:?} would end up in a log or a test failure, where anyone reading it could act as the
    // job's worker.
    #[test]
    fn debug_never_prints_the_job_token() {
        let inputs = inputs();
        let token = inputs.job_token.clone();
        let variables = worker_variables(&node(), &inputs);
        let launch = WorkerLaunch {
            environment_folder: PathBuf::from("/e/cpu"),
            job_file: PathBuf::from("/r/job.json"),
            run_folder: PathBuf::from("/r"),
            variables: variables.variables.clone(),
        };
        let printed = [
            format!("{inputs:?}"),
            format!("{variables:?}"),
            format!("{launch:?}"),
            format!("{launch:#?}"),
        ];
        for text in printed {
            assert!(!text.contains(&token), "{text}");
            assert!(text.contains(REDACTED_TOKEN), "{text}");
        }
        assert!(
            format!("{launch:?}").contains("RASTAI_RUN_ID"),
            "the other variables still show"
        );
    }

    #[test]
    fn a_tool_is_found_in_the_first_folder_that_has_it() {
        let folder = std::env::temp_dir().join(format!("rastai-spawn-path-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&folder);
        let tool = folder.join("tool-name");
        std::fs::write(&tool, "").unwrap();
        let path_value = std::env::join_paths([PathBuf::from("/nonexistent-folder"), folder.clone()]).unwrap();
        assert_eq!(find_on_path("tool-name", &path_value), Some(tool));
        assert_eq!(find_on_path("no-such-tool", &path_value), None);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
