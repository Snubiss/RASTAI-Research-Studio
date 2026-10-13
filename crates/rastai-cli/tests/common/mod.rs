//! What the CLI's integration tests share: a fresh `RASTAI_HOME` per test, the real `rastai` binary run in it with
//! a time limit, its `--json` output read back, and a guard that stops any daemon a test started, on every exit
//! path, panics included (#244).
//!
//! Each test binary uses only the helpers it needs, so the rest look unused to that binary.
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// A short prefix keeps `<home>/runtime/daemon.sock` within the 107 bytes a Unix socket address holds.
const TEST_FOLDER_PREFIX: &str = "rcl";
/// No single invocation in these tests should take this long; one that does is stuck, and is killed.
const RUN_TIME_LIMIT_SECONDS: u64 = 120;
const POLL_MS: u64 = 50;
/// The daemon a test starts picks any free port, so tests never fight over 47820–47829 or a real daemon's port.
const PORT_VARIABLE: &str = "RASTAI_DAEMON__PORT";
const ANY_FREE_PORT: &str = "0";
const HOME_VARIABLE: &str = "RASTAI_HOME";
/// A daemon gets this long to stop after `system.shutdown` before the guard kills it.
const SHUTDOWN_WAIT_SECONDS: u64 = 20;

/// What one run of the binary gave.
#[derive(Debug)]
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// An empty `RASTAI_HOME` for one test, under the system's temporary folder.
pub fn test_home(test_name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("{TEST_FOLDER_PREFIX}-{}-{test_name}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    home
}

pub fn daemon_socket(home: &Path) -> PathBuf {
    home.join("runtime").join("daemon.sock")
}

/// Runs `rastai <arguments>` with `RASTAI_HOME` set to `home`, in `working_folder`.
pub fn run_rastai_in(home: &Path, working_folder: &Path, arguments: &[&str]) -> RunResult {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rastai"))
        .args(arguments)
        .current_dir(working_folder)
        .env(HOME_VARIABLE, home)
        .env(PORT_VARIABLE, ANY_FREE_PORT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read both pipes on their own threads, so a chatty child never blocks on a full pipe.
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || read_all(&mut stdout_pipe));
    let stderr_reader = std::thread::spawn(move || read_all(&mut stderr_pipe));
    let status = wait_with_limit(&mut child, arguments);
    RunResult {
        exit_code: status,
        stdout: stdout_reader.join().unwrap(),
        stderr: stderr_reader.join().unwrap(),
    }
}

pub fn run_rastai(home: &Path, arguments: &[&str]) -> RunResult {
    run_rastai_in(home, home, arguments)
}

fn read_all(pipe: &mut impl Read) -> String {
    let mut text = String::new();
    let _ = pipe.read_to_string(&mut text);
    text
}

fn wait_with_limit(child: &mut std::process::Child, arguments: &[&str]) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(RUN_TIME_LIMIT_SECONDS);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().expect("rastai was killed by a signal");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("rastai {arguments:?} ran longer than {RUN_TIME_LIMIT_SECONDS} s");
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// The final `--json` line: `{"exit_code", "diagnostics"}`.
pub fn final_line(result: &RunResult) -> Value {
    let last = result.stdout.lines().last().unwrap_or_default();
    serde_json::from_str(last).unwrap_or_else(|_| panic!("no final JSON line in {result:?}"))
}

/// The codes of the final line's diagnostics, in order.
pub fn final_codes(result: &RunResult) -> Vec<String> {
    let line = final_line(result);
    let listed = line["diagnostics"].as_array().cloned().unwrap_or_default();
    listed
        .iter()
        .map(|item| item["code"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Stops the daemon a test started in `home` when it goes out of scope, and removes `home`: `system.shutdown`
/// first, then SIGKILL to the process group of any `rastai serve` still running with this `RASTAI_HOME`.
pub struct DaemonGuard {
    pub home: PathBuf,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if daemon_answers(&self.home) {
            let _ = run_rastai(&self.home, &["system", "shutdown", "--no-start", "--json"]);
            wait_until_stopped(&self.home);
        }
        for process_id in daemon_processes(&self.home) {
            kill_process_group(process_id);
        }
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// Whether a daemon answers on `home`'s socket.
pub fn daemon_answers(home: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(daemon_socket(home)).is_ok()
}

fn wait_until_stopped(home: &Path) {
    let deadline = Instant::now() + Duration::from_secs(SHUTDOWN_WAIT_SECONDS);
    while Instant::now() < deadline && !daemon_processes(home).is_empty() {
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// The `rastai` processes whose environment names this `RASTAI_HOME`: the daemon a verb started, which leads its own
/// process group. Linux only: it reads /proc.
pub fn daemon_processes(home: &Path) -> Vec<u32> {
    let wanted = format!("{HOME_VARIABLE}={}", home.display());
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Ok(process_id) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(environment) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        let names_home = environment
            .split(|byte| *byte == 0)
            .any(|pair| pair == wanted.as_bytes());
        if names_home && process_id != std::process::id() {
            found.push(process_id);
        }
    }
    found
}

fn kill_process_group(process_id: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{process_id}")])
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-KILL", &process_id.to_string()])
        .stderr(Stdio::null())
        .status();
}
