//! The test-only `RASTAI_TEST_PAUSE_AT` hook (contracts 15.9; flows/platform.md 6.5): the kill -9 matrix in
//! `tests/crash/` names a point in the daemon, the embedded node or the worker, and the process that reaches it stops
//! itself there, so the test's SIGKILL lands at that point every time instead of somewhere near it.
//!
//! - **Stopped, not sleeping.** The process sends itself SIGSTOP, so every thread freezes: no other task of the
//!   daemon commits a batch or answers a call after the point. The test sees the stop in `/proc/<pid>/stat` (state
//!   `T`), kills the process, or lets it go on with SIGCONT. The stop goes through the `kill` tool, as every signal of
//!   this crate does (jobs/signals.rs): `libc` would need `unsafe`.
//! - **The thread at the point stops first.** The signal is addressed to the id of the thread that reached the point,
//!   not to the process id (see `stop_this_process`), so that thread never runs past the point.
//! - **Once per process.** A process let go with SIGCONT runs past the point the next time it reaches it.
//! - **Debug builds only.** Every test build is one; a release build compiles `pause_at` to nothing and never hands
//!   the variable to a worker, so a shipped daemon never stops itself whatever its environment says. The worker's
//!   half is `rastai_worker/pause_points.py`; it only ever sees the variable from a debug node or a test.
//!
//! The points of the daemon, the node and the worker are listed in contracts 15.9, beside the variable.

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, Ordering};

/// The variable that names the point (contracts 15.9, tests only).
pub const TEST_PAUSE_VARIABLE: &str = "RASTAI_TEST_PAUSE_AT";
/// The procps tool that sends this process SIGSTOP without `unsafe` code.
#[cfg(debug_assertions)]
const KILL_TOOL: &str = "kill";
#[cfg(debug_assertions)]
const STOP_SIGNAL: &str = "STOP";
/// Links to `<pid>/task/<tid>` for whichever thread reads it (Linux).
#[cfg(debug_assertions)]
const THREAD_SELF_LINK: &str = "/proc/thread-self";

/// Set once this process has stopped at its point, so it doesn't stop there again after SIGCONT.
#[cfg(debug_assertions)]
static PAUSED_ALREADY: AtomicBool = AtomicBool::new(false);

/// Stops this process here when `RASTAI_TEST_PAUSE_AT` names `point` and it hasn't stopped at it yet. Returns once
/// the process is let go again (SIGCONT); a test usually kills it instead.
#[cfg(debug_assertions)]
pub fn pause_at(point: &str) {
    let configured = std::env::var(TEST_PAUSE_VARIABLE).ok();
    if !is_pause_point(configured.as_deref(), point) || PAUSED_ALREADY.swap(true, Ordering::SeqCst) {
        return;
    }
    stop_this_process(point);
}

/// A release build never pauses.
#[cfg(not(debug_assertions))]
pub fn pause_at(_point: &str) {}

/// Whether the configured value names this point. An unset or empty variable names none.
pub fn is_pause_point(configured: Option<&str>, point: &str) -> bool {
    match configured {
        Some(value) => !value.is_empty() && value == point,
        None => false,
    }
}

/// The variable as a worker should get it, from the node's own environment: a worker point stops the worker, and
/// the node clears every other variable before it starts one (jobs/spawn.rs). None in a release build.
pub fn worker_test_pause_variable(node_variables: &[(String, String)]) -> Option<(String, String)> {
    worker_test_pause_variable_for(cfg!(debug_assertions), node_variables)
}

/// The decision of `worker_test_pause_variable`, with the build given: the tests check both answers in every build.
pub fn worker_test_pause_variable_for(
    build_is_debug: bool,
    node_variables: &[(String, String)],
) -> Option<(String, String)> {
    if !build_is_debug {
        return None;
    }
    node_variables
        .iter()
        .find(|(name, value)| name == TEST_PAUSE_VARIABLE && !value.is_empty())
        .cloned()
}

/// SIGSTOP to this process. The line on stderr is for the test's output file, so a failing row shows where its
/// processes stopped.
///
/// SIGSTOP stops the whole process whichever of its ids it is sent to, but Linux hands it to the thread it was
/// addressed to, which then stops before it runs another line and makes the others stop too. Sent to the process id,
/// the main thread took it, and under load this thread came back from waiting for the tool and committed its batch
/// before the main thread got round to stopping the rest (row 2 of the kill -9 matrix found the batch indexed). So
/// it goes to this thread's own id; without `/proc/thread-self` (not Linux) it goes to the process id as before.
#[cfg(debug_assertions)]
fn stop_this_process(point: &str) {
    let pid = std::process::id().to_string();
    let target = match calling_thread_id() {
        Some(thread_id) => thread_id,
        None => pid.clone(),
    };
    eprintln!("{TEST_PAUSE_VARIABLE}: process {pid} paused at {point}");
    let stopped = std::process::Command::new(KILL_TOOL)
        .args(["-s", STOP_SIGNAL, &target])
        .status();
    if !matches!(stopped, Ok(status) if status.success()) {
        eprintln!("{TEST_PAUSE_VARIABLE}: process {pid} couldn't stop itself at {point}: {stopped:?}");
    }
}

/// The id of the thread that calls this, the last part of the `/proc/thread-self` link. None where there is no such
/// link.
#[cfg(debug_assertions)]
fn calling_thread_id() -> Option<String> {
    let link = std::fs::read_link(THREAD_SELF_LINK).ok()?;
    let thread_id = link.file_name()?.to_str()?;
    Some(thread_id.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The stop is addressed to the thread at the point: on a thread of its own, that is its id, one of this
    /// process's tasks, and not the process id the main thread has.
    // The stop itself is debug-only (a release build never stops), so the thread id it is addressed to is too.
    #[cfg(all(target_os = "linux", debug_assertions))]
    #[test]
    fn the_stop_goes_to_the_calling_thread() {
        let (thread_id, is_a_task_of_this_process) = std::thread::spawn(id_and_task_check).join().unwrap();
        assert_ne!(thread_id, std::process::id().to_string());
        assert!(is_a_task_of_this_process);
    }

    /// Run on the thread itself: a thread's task entry goes once it has ended.
    #[cfg(all(target_os = "linux", debug_assertions))]
    fn id_and_task_check() -> (String, bool) {
        let thread_id = calling_thread_id().unwrap();
        let is_a_task_of_this_process = std::path::Path::new("/proc/self/task").join(&thread_id).exists();
        (thread_id, is_a_task_of_this_process)
    }

    #[test]
    fn only_the_named_point_pauses() {
        assert!(is_pause_point(Some("daemon.journal.started"), "daemon.journal.started"));
        assert!(!is_pause_point(
            Some("daemon.journal.started"),
            "daemon.run_tail.before_commit"
        ));
        assert!(!is_pause_point(Some(""), ""));
        assert!(!is_pause_point(None, "daemon.journal.started"));
    }

    const WORKER_POINT: &str = "worker.event_log.mid_metrics_frame";
    /// The point the child process of the release test names. It only calls `pause_at` for it.
    #[cfg(not(debug_assertions))]
    const NAMED_POINT: &str = "daemon.journal.started";
    #[cfg(not(debug_assertions))]
    const NAMED_POINT_CHILD_TEST: &str = "pause_points::tests::named_point_child";
    /// How long the release test waits for its child: a child that stopped itself never exits, so this is what
    /// catches it.
    #[cfg(not(debug_assertions))]
    const CHILD_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
    #[cfg(not(debug_assertions))]
    const CHILD_POLL: std::time::Duration = std::time::Duration::from_millis(20);

    fn node_with_point() -> Vec<(String, String)> {
        vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            (TEST_PAUSE_VARIABLE.to_string(), WORKER_POINT.to_string()),
        ]
    }

    // Both answers of the build decision are checked in every build, so the release answer (none) is what every
    // `just check` sees, not only a release run's.
    #[test]
    fn the_build_decides_whether_a_worker_gets_the_point() {
        let expected = Some((TEST_PAUSE_VARIABLE.to_string(), WORKER_POINT.to_string()));
        assert_eq!(worker_test_pause_variable_for(true, &node_with_point()), expected);
        assert_eq!(worker_test_pause_variable_for(false, &node_with_point()), None);
    }

    // Test builds are debug builds: the node hands the point to its workers, and nothing else of its environment.
    #[cfg(debug_assertions)]
    #[test]
    fn a_debug_node_hands_the_point_to_its_workers() {
        let variables = node_with_point();
        assert_eq!(
            worker_test_pause_variable(&variables),
            Some((TEST_PAUSE_VARIABLE.to_string(), WORKER_POINT.to_string()))
        );
        let empty = vec![(TEST_PAUSE_VARIABLE.to_string(), String::new())];
        assert_eq!(worker_test_pause_variable(&empty), None);
        assert_eq!(worker_test_pause_variable(&variables[..1]), None);
    }

    // The release twin: a shipped node never hands the point to a worker, whatever its environment says.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_node_never_hands_the_point_to_its_workers() {
        assert_eq!(worker_test_pause_variable(&node_with_point()), None);
    }

    /// The release twin of the pause test: a release build's `pause_at` returns without stopping, even with the
    /// variable naming its point. Edition 2024 makes `set_var` unsafe, so the variable goes to a child test process,
    /// which calls `pause_at` (see `named_point_child`). A stopped child never exits, so the wait gives up.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_never_stops_even_when_the_point_is_named() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([NAMED_POINT_CHILD_TEST, "--exact", "--nocapture"])
            .env(TEST_PAUSE_VARIABLE, NAMED_POINT)
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "the child's pause_at didn't return: {status}");
                return;
            }
            if started.elapsed() > CHILD_WAIT {
                let _ = child.kill();
                panic!("the child didn't return from pause_at within {CHILD_WAIT:?}");
            }
            std::thread::sleep(CHILD_POLL);
        }
    }

    /// Run only by `a_release_build_never_stops_even_when_the_point_is_named`, with the variable naming the point. It
    /// passes on its own too: without the variable `pause_at` returns as well.
    #[cfg(not(debug_assertions))]
    #[test]
    fn named_point_child() {
        pause_at(NAMED_POINT);
    }
}
