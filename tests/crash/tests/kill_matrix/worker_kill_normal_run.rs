//! Row 13 (flows/platform.md 5.5 and 6.5): kill -9 the worker during a normal run, between two records. The node,
//! which didn't send the signal, fails the job with `RRS-J1001` naming SIGKILL; the daemon ends the run `failed`
//! after its tail read the log to the end; the worker's process group is gone. M1 jobs hold no VRAM lease
//! (`vram_leases.rs` comes with the inference step), so there is no lease to see released yet.

use crate::kill_matrix::checks::{LogEnd, check_after_row, job_failure, run_log, wait_until_run_indexed};
use crate::kill_matrix::commands::{start_training, wait_for_job_state};
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::processes::kill_process;
use crate::kill_matrix::scenario::Scenario;
use crate::kill_matrix::worker_kill_mid_frame::wait_for_paused_worker;

const PAUSE_POINT: &str = "worker.event_log.after_metrics_frame";
const CODE_KILLED_BY_SIGNAL: &str = "RRS-J1001";
const SIGNAL_NAME: &str = "SIGKILL";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_13_a_worker_killed_mid_run_fails_its_job_with_j1001_naming_sigkill() {
    let scenario = Scenario::new("row13");
    let daemon = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    let (run_id, job_id) = start_training(&scenario.paths, &scenario.training_arguments());
    let worker = wait_for_paused_worker(&scenario, &run_id);
    kill_process(worker.pid);

    wait_for_job_state(&scenario, &job_id, "failed");
    let failure = job_failure(&scenario.paths, &job_id).unwrap();
    assert_eq!(failure.code, CODE_KILLED_BY_SIGNAL, "{failure:?}");
    assert!(failure.message.contains(SIGNAL_NAME), "{failure:?}");
    wait_until_run_indexed(&scenario, &run_id, "failed");
    assert_eq!(run_log(&scenario.paths, &run_id).end, LogEnd::Whole);
    check_after_row(&scenario, &[&run_id]);
    daemon.stop(&scenario);
    scenario.finish();
}
