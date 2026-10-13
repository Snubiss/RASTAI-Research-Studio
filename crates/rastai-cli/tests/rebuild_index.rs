//! `rastai storage rebuild-index` against an index the real daemon wrote (plan M1-33; layout rust-control-plane.md
//! 13.2, flows/platform.md 6.4): the daemon indexes a data folder live, it stops, the binary rebuilds `rastai.db`
//! from the run folders, the daemon log and the blobs, and every rebuildable table matches the live one row by row.
//! A rebuild killed at any of its pause points leaves an index the store opens with every row it had, and the next
//! rebuild finishes. Linux only, like the daemon guard.
//!
//! The quick tests run a `train.start` that fails before it starts (the data folder has no cpu environment, so the
//! embedded node lacks what the job needs, `RRS-N0004`), plus tokens and a setting: runs, jobs, the journal, tokens,
//! nodes and the daemon log, with no Python. The `#[ignore]`d one trains for real in the cpu environment, so the run
//! log's states, metrics and checkpoints are compared too; `just test-daemon-jobs` runs it.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rastai_core::paths::{
    DataPaths, environments_dir, rastai_db_path, rastai_db_rebuilding_path, rastai_db_salvage_path,
};
use rastai_store::index::reads::{TableRows, list_every_table_rows, open_index_reader};
use rastai_store::rebuild::{
    PAUSE_AFTER_RUN_FOLDERS, PAUSE_BEFORE_RENAME, PAUSE_BEFORE_SWAP, rebuilt_table_differences,
};
use rastai_store::store::{close_store, open_store};
use serde_json::Value;

use common::{DaemonGuard, daemon_processes, final_codes, final_line, run_rastai, test_home};

const DESIGN: &str = "templates/designs/textcnn.rrsmodel";
const DATASET: &str = "fixtures/datasets/textcnn_tiny.jsonl";
/// The cpu environment's interpreter (`just env-cpu`), and where the embedded node looks for the environment
/// (`<data>/envs/<environment_id>-<lock_hash12>/`, as rastai-server's tests/train_jobs.rs links it).
const CPU_PYTHON: &str = ".dev/envs/cpu/bin/python";
const ENVIRONMENT_FOLDER: &str = "cpu-000000000000";
const EXECUTABLE_MODE: u32 = 0o755;
const ENDED_JOB_STATES: &[&str] = &["succeeded", "failed", "cancelled"];
const SUCCEEDED_STATE: &str = "succeeded";
/// A job without its environment fails at once; a real training run takes a few minutes on a loaded machine.
const QUICK_JOB_LIMIT: Duration = Duration::from_secs(60);
const TRAINING_LIMIT: Duration = Duration::from_secs(600);
const STOP_LIMIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
const HOME_VARIABLE: &str = "RASTAI_HOME";
const PAUSE_VARIABLE: &str = "RASTAI_TEST_PAUSE_AT";
/// In /proc/<pid>/stat, a process stopped by SIGSTOP, where the pause point leaves the rebuild.
const STOPPED_STATE: &str = "T";
const PAUSE_POINTS: &[&str] = &[PAUSE_AFTER_RUN_FOLDERS, PAUSE_BEFORE_SWAP, PAUSE_BEFORE_RENAME];

fn repository_file(relative: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
    path.canonicalize().unwrap()
}

/// The data folders `RASTAI_HOME=<home>` gives (contracts 8.1).
fn home_paths(home: &Path) -> DataPaths {
    DataPaths {
        config: home.join("config"),
        data: home.join("data"),
        cache: home.join("cache"),
        state: home.join("state"),
        runtime: home.join("runtime"),
    }
}

// ---- Making a data folder with the real daemon ----

/// `train.start --detach`: the job's id.
fn start_training(home: &Path) -> String {
    let design = repository_file(DESIGN).display().to_string();
    let dataset = repository_file(DATASET).display().to_string();
    let arguments = [
        "train",
        "start",
        "--recipe",
        "classification",
        "--design-path",
        &design,
        "--dataset-path",
        &dataset,
        "--detach",
        "--json",
    ];
    let result = run_rastai(home, &arguments);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let answer: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    answer["job_id"].as_str().unwrap().to_string()
}

/// Waits until `job.get` reads the job in a final state, and returns that state.
fn wait_until_ended(home: &Path, job_id: &str, limit: Duration) -> String {
    let started = Instant::now();
    loop {
        let result = run_rastai(home, &["job", "get", "--job-id", job_id, "--no-start", "--json"]);
        let answer: Value = serde_json::from_str(result.stdout.lines().next().unwrap_or_default()).unwrap_or_default();
        let state = answer["job"]["state"].as_str().unwrap_or_default().to_string();
        if ENDED_JOB_STATES.contains(&state.as_str()) {
            return state;
        }
        assert!(started.elapsed() < limit, "the job never ended: {result:?}");
        std::thread::sleep(POLL);
    }
}

/// `<data>/envs/cpu-.../bin/python` runs the cpu environment's interpreter with the node's arguments.
fn link_cpu_environment(paths: &DataPaths) {
    let python = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(CPU_PYTHON);
    assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
    let bin = environments_dir(paths).join(ENVIRONMENT_FOLDER).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!("#!/bin/sh\nexec '{}' \"$@\"\n", python.display());
    let launcher = bin.join("python");
    std::fs::write(&launcher, script).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
}

/// Journaled commands besides the run's: two tokens, one revoked, and a setting.
fn run_other_commands(home: &Path) {
    let created = run_rastai(
        home,
        &["token", "create", "--name", "kept", "--preset", "chat-only", "--json"],
    );
    assert_eq!(created.exit_code, 0, "{created:?}");
    let revoked = run_rastai(
        home,
        &["token", "create", "--name", "gone", "--preset", "chat-only", "--json"],
    );
    assert_eq!(revoked.exit_code, 0, "{revoked:?}");
    let answer: Value = serde_json::from_str(revoked.stdout.lines().next().unwrap()).unwrap();
    let token_id = answer["token_id"].as_str().unwrap();
    let revoke = run_rastai(home, &["token", "revoke", "--token-id", token_id, "--json"]);
    assert_eq!(revoke.exit_code, 0, "{revoke:?}");
    let setting_arguments = r#"{"key": "ui.density", "value": "compact"}"#;
    let setting = run_rastai(
        home,
        &["settings", "set", "--arguments-json", setting_arguments, "--json"],
    );
    assert_eq!(setting.exit_code, 0, "{setting:?}");
}

/// Stops the daemon with `system.shutdown` and waits until its process is gone, so its index is closed.
fn stop_daemon(home: &Path) {
    let result = run_rastai(home, &["system", "shutdown", "--no-start", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let started = Instant::now();
    while !daemon_processes(home).is_empty() {
        assert!(started.elapsed() < STOP_LIMIT, "the daemon didn't stop");
        std::thread::sleep(POLL);
    }
}

/// A data folder the daemon indexed live: a run whose job failed before it started, tokens and a setting.
fn data_folder_indexed_live(home: &Path) {
    let job_id = start_training(home);
    wait_until_ended(home, &job_id, QUICK_JOB_LIMIT);
    run_other_commands(home);
    stop_daemon(home);
}

// ---- Reading and comparing indexes ----

/// Every table of the index in `paths`, read through the store (which checks the file as the daemon does).
fn read_index(paths: &DataPaths) -> Vec<TableRows> {
    let store = open_store(paths).unwrap();
    let tables = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        list_every_table_rows(&reader).unwrap()
    };
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(close_store(store));
    tables
}

/// Every rebuildable table of `rebuilt` equals `live` row by row; the tables a rebuild starts empty are empty.
fn assert_rebuilt_matches(live: &[TableRows], rebuilt: &[TableRows]) {
    assert_eq!(rebuilt_table_differences(live, rebuilt), Vec::<String>::new());
}

fn row_count(tables: &[TableRows], table_name: &str) -> usize {
    let found = tables.iter().find(|table| table.table_name == table_name).unwrap();
    found.rows.len()
}

// ---- Running the rebuild ----

/// `rastai storage rebuild-index --json`, which must succeed; its result line.
fn rebuild(home: &Path) -> Value {
    let result = run_rastai(home, &["storage", "rebuild-index", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    assert_eq!(final_codes(&result), Vec::<String>::new(), "{result:?}");
    assert!(daemon_processes(home).is_empty(), "rebuild-index started a daemon");
    serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap()
}

/// A rebuild that stops itself at `point`; killed when dropped, on every exit path.
struct PausedRebuild {
    child: Child,
}

impl Drop for PausedRebuild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `rastai storage rebuild-index` with the pause point set and waits until it has stopped there.
fn rebuild_paused_at(home: &Path, point: &str) -> PausedRebuild {
    let child = Command::new(env!("CARGO_BIN_EXE_rastai"))
        .args(["storage", "rebuild-index", "--json"])
        .current_dir(home)
        .env(HOME_VARIABLE, home)
        .env(PAUSE_VARIABLE, point)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let paused = PausedRebuild { child };
    let started = Instant::now();
    while process_state(paused.child.id()).as_deref() != Some(STOPPED_STATE) {
        assert!(
            started.elapsed() < QUICK_JOB_LIMIT,
            "the rebuild never paused at {point}"
        );
        std::thread::sleep(POLL);
    }
    paused
}

/// The state letter of `/proc/<pid>/stat`, the first field after the command name.
fn process_state(process_id: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{process_id}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    after_name.split_whitespace().next().map(str::to_string)
}

// ---- The tests ----

#[test]
fn a_rebuilt_index_matches_the_live_one_row_by_row() {
    let home = test_home("rebuild-live");
    let _guard = DaemonGuard { home: home.clone() };
    data_folder_indexed_live(&home);
    let paths = home_paths(&home);
    let live = read_index(&paths);
    assert_eq!(row_count(&live, "runs"), 1);
    assert_eq!(row_count(&live, "jobs"), 1);
    assert_eq!(row_count(&live, "tokens"), 2);

    let report = rebuild(&home);

    assert_eq!(report["runs"], 1);
    let old_index = PathBuf::from(report["old_index_path"].as_str().unwrap());
    assert!(old_index.exists(), "the old index wasn't kept");
    assert!(!rastai_db_rebuilding_path(&paths).exists());
    assert_rebuilt_matches(&live, &read_index(&paths));
}

#[test]
fn a_rebuild_killed_at_any_point_leaves_a_usable_index() {
    let home = test_home("rebuild-kill");
    let _guard = DaemonGuard { home: home.clone() };
    data_folder_indexed_live(&home);
    let paths = home_paths(&home);
    let live = read_index(&paths);

    for point in PAUSE_POINTS {
        let paused = rebuild_paused_at(&home, point);
        drop(paused);
        // The old index is still at rastai.db, whole: the store opens it and every row is the live one.
        assert!(rastai_db_path(&paths).exists(), "no index after a kill at {point}");
        assert_eq!(read_index(&paths), live, "the index changed after a kill at {point}");
    }

    rebuild(&home);
    assert!(!rastai_db_rebuilding_path(&paths).exists());
    assert!(!rastai_db_salvage_path(&paths).exists());
    assert_rebuilt_matches(&live, &read_index(&paths));
}

/// The real worker trains to the end, so the run log's states, metrics and checkpoints are rebuilt and compared.
#[test]
#[ignore = "needs the cpu environment (just env-cpu); just test-daemon-jobs runs it"]
fn a_rebuilt_index_matches_a_trained_run_row_by_row() {
    let home = test_home("rebuild-trained");
    let _guard = DaemonGuard { home: home.clone() };
    let paths = home_paths(&home);
    link_cpu_environment(&paths);
    let job_id = start_training(&home);
    assert_eq!(wait_until_ended(&home, &job_id, TRAINING_LIMIT), SUCCEEDED_STATE);
    run_other_commands(&home);
    stop_daemon(&home);
    let live = read_index(&paths);
    assert!(row_count(&live, "run_metrics") > 0, "the run logged no metrics");
    assert!(row_count(&live, "checkpoints") > 0, "the run saved no checkpoint");

    let report = rebuild(&home);

    assert_eq!(report["runs"], 1);
    assert_rebuilt_matches(&live, &read_index(&paths));
    let shown = final_line(&run_rastai(&home, &["job", "get", "--job-id", &job_id, "--json"]));
    assert_eq!(shown["exit_code"], 0, "{shown:?}");
}
