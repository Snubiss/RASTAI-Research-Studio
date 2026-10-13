//! Row 2 (flows/platform.md 6.2 and 6.5): kill -9 the daemon after its run tail received a batch of records and
//! before the transaction that indexes them. The ack stays where it was, so the restarted daemon watches after it,
//! receives the same records again and indexes each exactly once.

use crate::kill_matrix::checks::{check_after_row, run_log, run_row, wait_until_run_indexed};
use crate::kill_matrix::commands::{start_training, wait_for_job_state};
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::scenario::Scenario;

const PAUSE_POINT: &str = "daemon.run_tail.before_commit";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_2_records_received_before_the_commit_are_indexed_exactly_once() {
    let scenario = Scenario::new("row2");
    let first = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    let (run_id, job_id) = start_training(&scenario.paths, &scenario.training_arguments());
    first.wait_until_paused(&scenario);
    // The daemon holds records it received and never committed: the log is ahead of the ack.
    let acked_at_kill = run_row(&scenario.paths, &run_id).unwrap().acked_sequence;
    assert!(run_log(&scenario.paths, &run_id).last_sequence() > acked_at_kill);
    first.kill_9();

    let second = DaemonProcess::start(&scenario, None);
    wait_for_job_state(&scenario, &job_id, "succeeded");
    wait_until_run_indexed(&scenario, &run_id, "completed");
    check_after_row(&scenario, &[&run_id]);
    second.stop(&scenario);
    scenario.finish();
}
