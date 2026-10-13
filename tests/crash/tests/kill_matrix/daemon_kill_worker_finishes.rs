//! Row 3 (flows/platform.md 2.3 and 6.5): kill -9 the daemon, and its embedded node, while TextCNN trains, and let the
//! worker finish while no daemon runs. Nothing waited on the worker, so its end comes from its log: the restarted
//! node finds the process gone and the last record a final run state, and the job is `succeeded` from that record.
//!
//! The embedded node stops the daemon process right after it streamed a `run.metrics` record to the daemon, so the
//! kill lands with the run training (rastai-node's run_events.rs).

use rastai_proto::common::v1::RunState;
use rastai_proto::events::v1::EventRecord;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;

use crate::kill_matrix::checks::{check_after_row, run_log, wait_until_run_indexed};
use crate::kill_matrix::commands::{job_summary, start_training, wait_for_job_state};
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::processes::{WorkerProcess, has_exited};
use crate::kill_matrix::scenario::{Scenario, run_folder};

const PAUSE_POINT: &str = "node.run_events.after_metrics_sent";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_3_a_worker_that_finishes_while_the_daemon_is_down_succeeds_from_its_final_record() {
    let scenario = Scenario::new("row3");
    let first = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    let (run_id, job_id) = start_training(&scenario.paths, &scenario.training_arguments());
    first.wait_until_paused(&scenario);
    let worker = WorkerProcess::from_job_file(&run_folder(&scenario.paths, &run_id)).unwrap();
    first.kill_9();

    scenario.wait_until("the worker to finish while no daemon runs", || has_exited(worker.pid));
    let log = run_log(&scenario.paths, &run_id);
    assert_eq!(
        final_run_state(&log.records),
        Some(RunState::Completed),
        "{}",
        scenario.describe()
    );

    let second = DaemonProcess::start(&scenario, None);
    wait_for_job_state(&scenario, &job_id, "succeeded");
    // No exit code: the restarted node isn't the worker's parent, so the end came from the log, not from a wait.
    let job = job_summary(&scenario.paths, &job_id);
    assert_eq!(job["error_code"].as_str().unwrap_or_default(), "", "{job}");
    assert!(job["exit_code"].is_null(), "{job}");
    wait_until_run_indexed(&scenario, &run_id, "completed");
    check_after_row(&scenario, &[&run_id]);
    second.stop(&scenario);
    scenario.finish();
}

/// The run state of the log's last record, when it is a `run.state_changed`.
fn final_run_state(records: &[EventRecord]) -> Option<RunState> {
    let last = records.last()?;
    let Some(Payload::RunStateChanged(changed)) = last.payload.as_ref().and_then(|payload| payload.payload.as_ref())
    else {
        return None;
    };
    match changed.state {
        Some(State::RunState(value)) => RunState::try_from(value).ok(),
        _ => None,
    }
}
