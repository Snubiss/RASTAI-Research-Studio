//! Processes as /proc shows them, and signals through procps' `kill`, as the node sends them (rastai-node's
//! jobs/signals.rs): Rust's std has no `kill(2)` for another process, and `libc` would need `unsafe`.
//!
//! A worker is known from its run's job.json, where the node records its pid and process group before anything
//! else can kill it. A worker leads its own group (`setsid`), so "no orphan process group" means no process is left
//! in that group.

use std::path::Path;
use std::process::Command;

const PROC_ROOT: &str = "/proc";
const STAT_FILE: &str = "stat";
const COMMAND_LINE_FILE: &str = "cmdline";
const JOB_FILE: &str = "job.json";
const KILL_TOOL: &str = "kill";
/// In /proc/<pid>/stat, after the command name's closing parenthesis: the state is the first field, the process
/// group the third.
const STATE_FIELD_AFTER_NAME: usize = 0;
const GROUP_FIELD_AFTER_NAME: usize = 2;
/// Stopped by a signal: where `RASTAI_TEST_PAUSE_AT` leaves a process.
const STOPPED_STATE: &str = "T";
/// A process that has exited and runs nothing more: a zombie its parent hasn't reaped yet, or one being torn down.
const EXITED_STATES: &[&str] = &["Z", "X", "x"];
/// What a worker's command line holds (`python -m rastai_worker run --job-file ...`), so a group is killed only
/// while it is still a worker's: a reused group id is left alone.
const WORKER_MODULE: &str = "rastai_worker";

/// A worker as its job.json records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerProcess {
    pub pid: u32,
    pub process_group_id: u32,
}

impl WorkerProcess {
    /// The worker the run folder's job.json records, or None before the node has started one.
    pub fn from_job_file(run_folder: &Path) -> Option<WorkerProcess> {
        let text = std::fs::read_to_string(run_folder.join(JOB_FILE)).ok()?;
        let fields: serde_json::Value = serde_json::from_str(&text).ok()?;
        let pid = u32::try_from(fields["pid"].as_u64()?).ok()?;
        let process_group_id = u32::try_from(fields["process_group_id"].as_u64()?).ok()?;
        if pid == 0 {
            return None;
        }
        Some(WorkerProcess { pid, process_group_id })
    }
}

/// The process's state letter, or None when /proc no longer lists it.
pub fn process_state(pid: u32) -> Option<String> {
    stat_field(pid, STATE_FIELD_AFTER_NAME)
}

/// Stopped at a pause point (or by a test's SIGSTOP).
pub fn is_stopped(pid: u32) -> bool {
    process_state(pid).as_deref() == Some(STOPPED_STATE)
}

/// Gone, or exited and waiting to be reaped: it runs nothing more.
pub fn has_exited(pid: u32) -> bool {
    match process_state(pid) {
        Some(state) => EXITED_STATES.contains(&state.as_str()),
        None => true,
    }
}

/// The pids of the processes in `process_group_id` that haven't exited.
pub fn live_group_members(process_group_id: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(PROC_ROOT) else {
        return Vec::new();
    };
    let mut members = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let group = stat_field(pid, GROUP_FIELD_AFTER_NAME);
        if group == Some(process_group_id.to_string()) && !has_exited(pid) {
            members.push(pid);
        }
    }
    members
}

/// SIGKILL to one process, as an outside kill would send it.
pub fn kill_process(pid: u32) {
    send_signal("KILL", &pid.to_string());
}

/// SIGSTOP or SIGCONT to a whole worker group: a test holds a worker still, or lets it go on.
pub fn stop_group(worker: &WorkerProcess) {
    send_signal("STOP", &group_target(worker));
}

pub fn continue_group(worker: &WorkerProcess) {
    send_signal("CONT", &group_target(worker));
}

/// SIGKILL to the worker's group, while it is still a worker's group: the clean-up of every exit path.
pub fn kill_worker_group(worker: &WorkerProcess) {
    let members = live_group_members(worker.process_group_id);
    if members.iter().any(|pid| is_worker(*pid)) {
        send_signal("KILL", &group_target(worker));
    }
}

fn is_worker(pid: u32) -> bool {
    let path = Path::new(PROC_ROOT).join(pid.to_string()).join(COMMAND_LINE_FILE);
    let command_line = std::fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(&command_line).contains(WORKER_MODULE)
}

/// `-<pgid>`: what `kill` takes for a group.
fn group_target(worker: &WorkerProcess) -> String {
    format!("-{}", worker.process_group_id)
}

/// `kill -s <signal> -- <target>`. A target that is already gone is not a failure: the clean-up and the rows send
/// signals to processes that may have just ended.
fn send_signal(signal: &str, target: &str) {
    let _ = Command::new(KILL_TOOL).args(["-s", signal, "--", target]).status();
}

/// One field of /proc/<pid>/stat, counted from the first after the command name. The name is in parentheses and may
/// hold spaces and parentheses itself, so fields are counted after its last `)`.
fn stat_field(pid: u32, field_after_name: usize) -> Option<String> {
    let path = Path::new(PROC_ROOT).join(pid.to_string()).join(STAT_FILE);
    let text = std::fs::read_to_string(path).ok()?;
    let after_name = &text[text.rfind(')')? + 1..];
    after_name.split_whitespace().nth(field_after_name).map(str::to_string)
}
