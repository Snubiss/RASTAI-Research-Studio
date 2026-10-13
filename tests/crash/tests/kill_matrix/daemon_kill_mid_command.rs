//! Row 9 (flows/platform.md 3.2 and 6.5): kill -9 the daemon between a command's journal rows, after the started row
//! committed and before the handler ran. The restarted daemon closes the row as interrupted, `RRS-C0005`, before it
//! serves anything, and the command left nothing else behind: `train.start` never ran, so there is no run and no job.

use std::thread;

use crate::kill_matrix::checks::{assert_catalog_code, check_after_row, table_rows, unquoted};
use crate::kill_matrix::commands::execute;
use crate::kill_matrix::daemon_process::DaemonProcess;
use crate::kill_matrix::scenario::Scenario;

const PAUSE_POINT: &str = "daemon.journal.started";
const CODE_INTERRUPTED: &str = "RRS-C0005";
const COMMAND_ID: &str = "train.start";
const IDEMPOTENCY_KEY: &str = "crash-row-9";

#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-crash runs it"]
fn row_9_a_command_cut_off_between_its_journal_rows_is_marked_interrupted() {
    let scenario = Scenario::new("row9");
    let first = DaemonProcess::start(&scenario, Some(PAUSE_POINT));
    // The request waits on the stopped daemon, so it is sent from a thread of its own; the kill ends it.
    let paths = scenario.paths.clone();
    let arguments = scenario.training_arguments();
    let request = thread::spawn(move || execute(&paths, COMMAND_ID, &arguments, IDEMPOTENCY_KEY));
    first.wait_until_paused(&scenario);
    first.kill_9();
    let cut_off = request.join().unwrap();
    assert!(
        cut_off.is_err(),
        "the command answered from a killed daemon: {cut_off:?}"
    );

    let second = DaemonProcess::start(&scenario, None);
    let journal = table_rows(&scenario.paths, "command_journal");
    // Columns: journal_id, command_id, principal, arguments_json, ok, code, idempotency_key, result_json, job_id,
    // started_unix_ms, finished_unix_ms (contracts 8.2).
    let rows: Vec<&Vec<String>> = journal.iter().filter(|row| unquoted(&row[1]) == COMMAND_ID).collect();
    assert_eq!(rows.len(), 1, "{journal:?}");
    let row = rows[0];
    assert_eq!(
        (row[4].as_str(), unquoted(&row[5]).as_str()),
        ("0", CODE_INTERRUPTED),
        "{row:?}"
    );
    assert_catalog_code(CODE_INTERRUPTED, "the interrupted command");
    assert_eq!(unquoted(&row[6]), IDEMPOTENCY_KEY);
    assert_ne!(unquoted(&row[10]), "", "the closed row has its finish time");
    assert!(
        table_rows(&scenario.paths, "runs").is_empty(),
        "train.start never ran, yet a run exists"
    );
    assert!(
        table_rows(&scenario.paths, "jobs").is_empty(),
        "train.start never ran, yet a job exists"
    );
    check_after_row(&scenario, &[]);
    second.stop(&scenario);
    scenario.finish();
}
