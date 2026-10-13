//! One row's world: a fresh folder with the daemon's five data roots, the cpu environment linked where the embedded
//! node looks for it, and the waits every row uses. Dropping a scenario kills every worker process group its runs
//! recorded (issue #244); `finish` removes the folder once the row passed, and a failed row leaves it to look at.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rastai_core::paths::{DataPaths, environments_dir, run_dir};

use crate::kill_matrix::processes::{WorkerProcess, kill_worker_group};

/// A short parent folder: a socket path must fit in 107 bytes, and jobs/<job_id>.sock sits under it.
const FOLDER_PREFIX: &str = "rcr";
/// The cpu environment's interpreter, the design and the dataset, relative to this crate.
const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
const DESIGN: &str = "../../templates/designs/textcnn.rrsmodel";
const DATASET: &str = "../../fixtures/datasets/textcnn_tiny.jsonl";
/// `<data>/envs/<environment_id>-<lock_hash12>/`: the embedded node finds the cpu environment here (daemon.rs).
const ENVIRONMENT_FOLDER: &str = "cpu-000000000000";
const EXECUTABLE_MODE: u32 = 0o755;
/// The budget guards against a hang, not a slow machine: a TextCNN run took 60 s at load average 90 (#234), and a
/// row waits on a start, a run and a restart.
pub const WAIT_LIMIT: Duration = Duration::from_secs(600);
pub const POLL: Duration = Duration::from_millis(50);
/// Where the daemon processes of the row write what they print.
pub const DAEMON_OUTPUT_FILE: &str = "daemon-output.txt";
const STDERR_LOG: &str = "stderr.log";
const RUNS_FOLDER: &str = "runs";
/// How much of each output a failed wait shows.
const OUTPUT_TAIL_BYTES: usize = 4000;

pub struct Scenario {
    pub folder: PathBuf,
    pub paths: DataPaths,
}

impl Drop for Scenario {
    fn drop(&mut self) {
        for worker in recorded_workers(&self.paths) {
            kill_worker_group(&worker);
        }
    }
}

impl Scenario {
    /// An empty folder for the row named `name`, with the cpu environment linked in.
    pub fn new(name: &str) -> Scenario {
        let folder = std::env::temp_dir().join(format!("{FOLDER_PREFIX}-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let scenario = Scenario {
            paths: paths_in(&folder),
            folder,
        };
        link_cpu_environment(&scenario.paths);
        scenario
    }

    /// The row passed: its folder goes. Called last, after the daemon stopped.
    pub fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }

    /// What the processes printed, for a failed wait: the daemon's output and each run's stderr.log, their ends.
    pub fn describe(&self) -> String {
        let mut text = format!("daemon output:\n{}", file_tail(&self.folder.join(DAEMON_OUTPUT_FILE)));
        for run_folder in run_folders(&self.paths) {
            let stderr = file_tail(&run_folder.join(STDERR_LOG));
            text.push_str(&format!("\n{}/{STDERR_LOG}:\n{stderr}", run_folder.display()));
        }
        text
    }

    /// Polls `condition` until it holds; fails the row with `what` and the processes' output after `WAIT_LIMIT`.
    pub fn wait_until(&self, what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < WAIT_LIMIT,
                "waited {WAIT_LIMIT:?} for {what}\n{}",
                self.describe()
            );
            std::thread::sleep(POLL);
        }
    }

    /// The arguments of a `train.start` of TextCNN on the tiny dataset.
    pub fn training_arguments(&self) -> serde_json::Value {
        serde_json::json!({
            "recipe": "classification",
            "design_path": crate_path(DESIGN),
            "dataset_path": crate_path(DATASET),
        })
    }
}

/// The five data roots in one folder, the way `RASTAI_HOME` lays them out.
fn paths_in(folder: &Path) -> DataPaths {
    DataPaths {
        config: folder.join("config"),
        data: folder.join("data"),
        cache: folder.join("cache"),
        state: folder.join("state"),
        runtime: folder.join("runtime"),
    }
}

/// `<data>/envs/cpu-.../bin/python` runs the cpu environment's interpreter with the node's arguments.
fn link_cpu_environment(paths: &DataPaths) {
    let python = crate_path(CPU_PYTHON);
    assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
    let bin = environments_dir(paths).join(ENVIRONMENT_FOLDER).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!("#!/bin/sh\nexec '{}' \"$@\"\n", python.display());
    let launcher = bin.join("python");
    std::fs::write(&launcher, script).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
}

/// A path relative to this crate. Never canonicalized: the cpu environment's `bin/python` is a link whose own path
/// is what makes Python use the environment.
fn crate_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Every run folder of the scenario's data directory.
pub fn run_folders(paths: &DataPaths) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(paths.data.join(RUNS_FOLDER)) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    folders.sort();
    folders
}

/// The worker each run's job.json records, once the node has started one.
pub fn recorded_workers(paths: &DataPaths) -> Vec<WorkerProcess> {
    let mut workers = Vec::new();
    for run_folder in run_folders(paths) {
        if let Some(worker) = WorkerProcess::from_job_file(&run_folder) {
            workers.push(worker);
        }
    }
    workers
}

/// The run's folder (`<data>/runs/<run_id>/`).
pub fn run_folder(paths: &DataPaths, run_id: &str) -> PathBuf {
    run_dir(paths, run_id)
}

fn file_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let start = text.len().saturating_sub(OUTPUT_TAIL_BYTES);
    let mut boundary = start;
    while !text.is_char_boundary(boundary) {
        boundary += 1;
    }
    text[boundary..].to_string()
}
