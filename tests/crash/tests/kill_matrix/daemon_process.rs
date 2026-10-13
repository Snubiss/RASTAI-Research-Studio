//! The daemon as a process of its own: the real `rastai serve --headless --port 0`, with `RASTAI_HOME` naming the
//! scenario's folder (and `RASTAI_TEST_PAUSE_AT` when the row names a point). The row waits until it serves, and
//! later waits for its pause, kills it or stops it cleanly with SIGTERM.
//!
//! The binary is the debug `rastai` next to this test binary's own folder, which `just test-crash` builds first; only
//! a debug build honours the pause points.
//!
//! Dropping a `DaemonProcess` kills it, so a failed row never leaves a daemon behind. Its embedded node goes with
//! it; the workers don't, which is what the scenario's own drop is for.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use crate::kill_matrix::commands::daemon_answers;
use crate::kill_matrix::processes::{has_exited, is_stopped};
use crate::kill_matrix::scenario::{DAEMON_OUTPUT_FILE, Scenario};

/// The CLI binary and the arguments that run the daemon without a browser. Port 0: the operating system picks a free
/// loopback port, so rows never fight over 47820–47829.
const RASTAI_BINARY: &str = "rastai";
const SERVE_ARGUMENTS: &[&str] = &["serve", "--headless", "--port", "0"];
/// `RASTAI_HOME` puts all five data roots in one folder (contracts 8.1), the layout `scenario::paths_in` makes.
const HOME_VARIABLE: &str = "RASTAI_HOME";
/// The hook's variable (rastai-node pause_points.rs; contracts 15.9).
const PAUSE_VARIABLE: &str = "RASTAI_TEST_PAUSE_AT";
const KILL_TOOL: &str = "kill";

/// A daemon process the row started.
pub struct DaemonProcess {
    process: Child,
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl DaemonProcess {
    /// Starts a daemon on the scenario's folder and waits until it serves daemon.sock. With `pause_at`, the daemon,
    /// its embedded node and its workers get `RASTAI_TEST_PAUSE_AT`; a point the daemon reaches before it serves
    /// can't be waited for this way, and no row names one.
    pub fn start(scenario: &Scenario, pause_at: Option<&str>) -> DaemonProcess {
        let daemon = DaemonProcess::spawn(scenario, pause_at);
        let pid = daemon.process.id();
        scenario.wait_until("the daemon to serve daemon.sock", || {
            assert!(!has_exited(pid), "the daemon exited\n{}", scenario.describe());
            daemon_answers(&scenario.paths)
        });
        daemon
    }

    fn spawn(scenario: &Scenario, pause_at: Option<&str>) -> DaemonProcess {
        let output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(scenario.folder.join(DAEMON_OUTPUT_FILE))
            .unwrap();
        let mut command = Command::new(rastai_binary());
        command
            .args(SERVE_ARGUMENTS)
            .env(HOME_VARIABLE, &scenario.folder)
            .env_remove(PAUSE_VARIABLE)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        if let Some(point) = pause_at {
            command.env(PAUSE_VARIABLE, point);
        }
        DaemonProcess {
            process: command.spawn().unwrap(),
        }
    }

    pub fn pid(&self) -> u32 {
        self.process.id()
    }

    /// Waits until the daemon process has stopped itself at its pause point.
    pub fn wait_until_paused(&self, scenario: &Scenario) {
        let pid = self.pid();
        scenario.wait_until("the daemon to stop at its pause point", || {
            assert!(!has_exited(pid), "the daemon exited\n{}", scenario.describe());
            is_stopped(pid)
        });
    }

    /// kill -9, and waits until the process is gone (reaped), so its lock and sockets are free to take.
    pub fn kill_9(mut self) {
        self.process.kill().unwrap();
        self.process.wait().unwrap();
    }

    /// SIGTERM, the clean stop of `rastai serve`, and waits for the process to end: the store closes and the lock is
    /// released. The row then reads nothing more from this daemon.
    pub fn stop(mut self, scenario: &Scenario) {
        let status = Command::new(KILL_TOOL)
            .args(["-s", "TERM", &self.pid().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let mut ended = None;
        scenario.wait_until("the daemon to stop after SIGTERM", || {
            ended = self.process.try_wait().unwrap();
            ended.is_some()
        });
        let ended = ended.unwrap();
        assert!(
            ended.success(),
            "the daemon's clean stop failed: {ended}\n{}",
            scenario.describe()
        );
    }
}

/// `target/<profile>/rastai`: this test binary is `target/<profile>/deps/kill_matrix-<hash>`, so the CLI built by
/// `cargo build -p rastai-cli` in the same profile sits two folders up from it.
fn rastai_binary() -> PathBuf {
    let test_binary = std::env::current_exe().unwrap();
    let profile_folder = test_binary.parent().and_then(|deps| deps.parent()).unwrap();
    let binary = profile_folder.join(RASTAI_BINARY);
    assert!(
        binary.exists(),
        "{} is missing: run `cargo build -p rastai-cli` first (`just test-crash` does)",
        binary.display()
    );
    binary
}
