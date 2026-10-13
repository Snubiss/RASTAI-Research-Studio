//! Row 11, the daemon's half (flows/platform.md 6.2 and 6.5): kill -9 the worker inside a frame write. The log ends
//! in a torn tail; readers stop before it and never repair it (only a writer truncates). The node ends the run's
//! stream at the last whole frame, the daemon indexes every whole record exactly once and nothing of the torn one,
//! and the job fails with `RRS-J1001` (a SIGKILL the node didn't send).
//!
//! The other half, `train.resume` truncating the torn tail and continuing the sequence, is the worker's: the next
//! job of the run opens the log as its writer. M1 has no `train.resume` command yet (M5), so
//! `tests/crash/test_worker_kill9.py` starts that job as the node would and checks it.

use crate::kill_matrix::checks::{LogEnd, check_after_row, job_failure, run_log, wait_until_run_indexed};
use crate::kill_matrix::commands::{start_training, wait_for_job_state};
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::processes::{WorkerProcess, has_exited, is_stopped, kill_process};
use crate::kill_matrix::scenario::{Scenario, run_folder};

const PAUSE_POINT: &str = "worker.event_log.mid_metrics_frame";
const CODE_KILLED_BY_SIGNAL: &str = "RRS-J1001";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_11_a_worker_killed_inside_a_frame_leaves_a_torn_tail_the_daemon_ignores() {
    let scenario = Scenario::new("row11");
    let daemon = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    let (run_id, job_id) = start_training(&scenario.paths, &scenario.training_arguments());
    let worker = wait_for_paused_worker(&scenario, &run_id);
    let torn = run_log(&scenario.paths, &run_id);
    assert_eq!(torn.end, LogEnd::TornTail, "{}", scenario.describe());
    kill_process(worker.pid);

    wait_for_job_state(&scenario, &job_id, "failed");
    let failure = job_failure(&scenario.paths, &job_id).unwrap();
    assert_eq!(failure.code, CODE_KILLED_BY_SIGNAL, "{failure:?}");
    wait_until_run_indexed(&scenario, &run_id, "failed");
    // Ignored, not repaired: the file still ends inside the frame, and the whole records before it are all there is.
    let after = run_log(&scenario.paths, &run_id);
    assert_eq!(
        (after.end, after.last_sequence()),
        (LogEnd::TornTail, torn.last_sequence())
    );
    check_after_row(&scenario, &[&run_id]);
    daemon.stop(&scenario);
    scenario.finish();
}

/// Waits until the run's worker has started and stopped itself at the pause point, and returns it.
pub fn wait_for_paused_worker(scenario: &Scenario, run_id: &str) -> WorkerProcess {
    let folder = run_folder(&scenario.paths, run_id);
    let mut worker = None;
    scenario.wait_until("the worker to start", || {
        worker = WorkerProcess::from_job_file(&folder);
        worker.is_some()
    });
    let worker = worker.unwrap();
    scenario.wait_until("the worker to stop at its pause point", || {
        assert!(!has_exited(worker.pid), "the worker ended\n{}", scenario.describe());
        is_stopped(worker.pid)
    });
    worker
}
