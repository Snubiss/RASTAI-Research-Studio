//! The test-only `RASTAI_TEST_PAUSE_AT` hook (contracts 15.9; flows/platform.md 6.5): the kill -9 matrix in
//! `tests/crash/` names a point in the daemon, the embedded node or the worker, and the process that reaches it stops
//! itself there, so the test's SIGKILL lands at that point every time instead of somewhere near it.
//!
//! - **Stopped, not sleeping.** The process sends itself SIGSTOP, so every thread freezes at once: no other task of
//!   the daemon commits a batch or answers a call after the point. The test sees the stop in `/proc/<pid>/stat`
//!   (state `T`), kills the process, or lets it go on with SIGCONT. The stop goes through the `kill` tool, as every
//!   signal of this crate does (jobs/signals.rs): `libc` would need `unsafe`.
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
    if !cfg!(debug_assertions) {
        return None;
    }
    node_variables
        .iter()
        .find(|(name, value)| name == TEST_PAUSE_VARIABLE && !value.is_empty())
        .cloned()
}

/// SIGSTOP to this process. The line on stderr is for the test's output file, so a failing row shows where its
/// processes stopped. The kill tool's own wait returns only after the stop is delivered and lifted again: the
/// kernel stops every thread before this one can return from waiting for the tool.
#[cfg(debug_assertions)]
fn stop_this_process(point: &str) {
    let pid = std::process::id();
    eprintln!("{TEST_PAUSE_VARIABLE}: process {pid} paused at {point}");
    let stopped = std::process::Command::new(KILL_TOOL)
        .args(["-s", STOP_SIGNAL, &pid.to_string()])
        .status();
    if !matches!(stopped, Ok(status) if status.success()) {
        eprintln!("{TEST_PAUSE_VARIABLE}: process {pid} couldn't stop itself at {point}: {stopped:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Test builds are debug builds: the node hands the point to its workers, and nothing else of its environment.
    #[test]
    fn a_debug_node_hands_the_point_to_its_workers() {
        let variables = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            (
                TEST_PAUSE_VARIABLE.to_string(),
                "worker.event_log.mid_metrics_frame".to_string(),
            ),
        ];
        assert_eq!(
            worker_test_pause_variable(&variables),
            Some((
                TEST_PAUSE_VARIABLE.to_string(),
                "worker.event_log.mid_metrics_frame".to_string()
            ))
        );
        let empty = vec![(TEST_PAUSE_VARIABLE.to_string(), String::new())];
        assert_eq!(worker_test_pause_variable(&empty), None);
        assert_eq!(worker_test_pause_variable(&variables[..1]), None);
    }
}
