//! Row 1, the §32 exit of weeks 3–4 (flows/platform.md 2.3 and 6.5; layout rust-node-gateway-proto.md 7.12): kill -9
//! the daemon, and with it the embedded node, while TextCNN trains. The worker trains on and writes records no daemon
//! reads. The restarted daemon's node takes the job back (`lost`, then `running` once the worker registers again),
//! the run's tail resumes after the ack the killed daemon committed, and in the end `run_metrics` is the log: every
//! metric once, no gap, no duplicate.
//!
//! The daemon stops itself right after it committed a batch holding `run.metrics`, so the kill lands with the run
//! training and indexed up to a known point. The worker is held (SIGSTOP from the row) once it has written records
//! past that point, so it is still running when the new daemon starts, however fast this machine trains, and let go
//! after: a held process is alive to the node's reattach check, and it registers again once it runs.

use crate::kill_matrix::checks::{check_after_row, run_log, wait_until_run_indexed};
use crate::kill_matrix::commands::{start_training, wait_for_job_state};
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::processes::{WorkerProcess, continue_group, has_exited, is_stopped, stop_group};
use crate::kill_matrix::scenario::{Scenario, run_folder};

const PAUSE_POINT: &str = "daemon.run_tail.after_metrics_commit";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_1_daemon_killed_mid_run_restarts_reattaches_and_loses_no_event() {
    let scenario = Scenario::new("row1");
    let first = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    let (run_id, job_id) = start_training(&scenario.paths, &scenario.training_arguments());
    first.wait_until_paused(&scenario);
    let worker = WorkerProcess::from_job_file(&run_folder(&scenario.paths, &run_id)).unwrap();
    let logged_at_kill = run_log(&scenario.paths, &run_id).last_sequence();
    first.kill_9();

    // No daemon and no node: the worker keeps training and writing. Records past the kill reach no index yet.
    scenario.wait_until("the worker to write records while no daemon runs", || {
        run_log(&scenario.paths, &run_id).last_sequence() > logged_at_kill
    });
    stop_group(&worker);
    scenario.wait_until("the worker to be held", || {
        assert!(
            !has_exited(worker.pid),
            "the worker ended before the restart\n{}",
            scenario.describe()
        );
        is_stopped(worker.pid)
    });

    let second = DaemonProcess::start(&scenario, None);
    // The new node found the job alive; it can't register while held, so it is lost, not gone (RRS-J3002).
    wait_for_job_state(&scenario, &job_id, "lost");
    continue_group(&worker);
    wait_for_job_state(&scenario, &job_id, "running");

    wait_for_job_state(&scenario, &job_id, "succeeded");
    wait_until_run_indexed(&scenario, &run_id, "completed");
    check_after_row(&scenario, &[&run_id]);
    second.stop(&scenario);
    scenario.finish();
}
