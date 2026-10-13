//! The daemon as a process of its own. The row's side starts this test binary again with
//! `RASTAI_TEST_DAEMON_FOLDER` (and `RASTAI_TEST_PAUSE_AT` when the row names a point), waits until it serves, and
//! later waits for its pause, kills it or stops it cleanly. The child's side, `serve_if_asked`, runs what `rastai
//! serve` runs on the folder's data roots: start, serve, and stop on SIGTERM.
//!
//! Dropping a `DaemonProcess` kills it, so a failed row never leaves a daemon behind. Its embedded node goes with
//! it; the workers don't, which is what the scenario's own drop is for.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use rastai_server::daemon::{DaemonOptions, start_daemon};
use rastai_server::error::ServerError;
use rastai_server::shutdown::{stop_on_signals, wait_for_shutdown};

use crate::kill_matrix::commands::daemon_answers;
use crate::kill_matrix::processes::{has_exited, is_stopped};
use crate::kill_matrix::scenario::{DAEMON_OUTPUT_FILE, Scenario, paths_in};

/// The variable that makes this binary a daemon process, naming its folder (contracts 15.9, tests only), and the
/// one test such a process runs.
const DAEMON_FOLDER_VARIABLE: &str = "RASTAI_TEST_DAEMON_FOLDER";
const DAEMON_PROCESS_TEST: &str = "kill_matrix::daemon_process_for_kill_tests";
/// The hook's variable (rastai-node pause_points.rs; contracts 15.9).
const PAUSE_VARIABLE: &str = "RASTAI_TEST_PAUSE_AT";
/// Port 0: the operating system picks a free loopback port, so rows never fight over 47820–47829.
const ANY_FREE_PORT: u16 = 0;
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
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([DAEMON_PROCESS_TEST, "--exact", "--nocapture"])
            .env(DAEMON_FOLDER_VARIABLE, &scenario.folder)
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

/// The child's side: with `RASTAI_TEST_DAEMON_FOLDER` set, runs a daemon on that folder until SIGTERM, as `rastai
/// serve` does; without it, returns at once.
pub fn serve_if_asked() {
    let Ok(folder) = std::env::var(DAEMON_FOLDER_VARIABLE) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let served = runtime.block_on(serve_until_stopped(Path::new(&folder)));
    if let Err(error) = served {
        panic!("the daemon stopped with {}: {error:?}", error.code());
    }
}

async fn serve_until_stopped(folder: &Path) -> Result<(), ServerError> {
    let options = DaemonOptions {
        port: Some(ANY_FREE_PORT),
        ..DaemonOptions::default()
    };
    let running = start_daemon(paths_in(folder), &options).await?;
    stop_on_signals(&running.server);
    wait_for_shutdown(running).await
}
