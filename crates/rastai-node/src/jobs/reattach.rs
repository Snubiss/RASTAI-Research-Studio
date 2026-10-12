//! Reattach after a node or daemon restart (layout: rust-node-gateway-proto.md section 7.4, "Reattach"; platform
//! flow 2.3). Jobs outlive the node: each worker runs in its own process group, so a node that was killed (or the
//! daemon that embeds it) leaves its jobs running, and they keep appending to their event logs. On start the node
//! reads every `runs/<run_id>/job.json` without a `final_state` and decides, per job:
//!
//! - **The process is still the job's** (`is_running_process`: same boot id, start ticks and process group, leading
//!   that group, not exited): the job enters the table `lost`, holding its run. The node stored only the token's hash,
//!   so it waits for the worker to `Register` again; the token's BLAKE3 must match `job_token_hash`, then `Hello`, then
//!   `running` (supervisor.rs `register_worker`). A worker that doesn't come back within
//!   `HOST_RECONNECT_TIMEOUT_SECONDS` is killed with its group (the pid checked again first) and fails with
//!   `RRS-J3003`. The job isn't the node's child, so its end is seen by polling `/proc` every
//!   `PROCESS_POLL_SECONDS`, and decided from the log (exit_cause.rs `classify_vanished`).
//! - **The process is gone, or the pid now names another process** (a reused pid, an unrelated long-lived one, a pid
//!   from an earlier boot): the node never signals it. The job ends at once from what it left: its last
//!   `run.state_changed` in the run's log; `RRS-J3006` if it left no record there and the run's `stderr.log` ends with
//!   that refusal (exit 5: the log is then another writer's); else `failed` with `RRS-J3002`. The terminal `job.json`
//!   is the one read back, with its final state.
//!
//! `node.rs` runs this before it serves `worker-host.sock`, so a worker that registers the moment the socket appears
//! always finds its job in the table: a `Register` for an unknown job is refused, and the worker takes a refusal as
//! final.
//!
//! A `job.json` that can't be read, or whose ids aren't ids, is logged and left alone: nothing in it can be trusted
//! enough to signal or to end. Not here yet: leases restored from `granted_leases` (M3), sandboxes adopted (M5), and
//! the node-native kinds (`download`, `transfer`: `RRS-J3004`, M2 and M9).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::paths::run_dir;
use rastai_core::proto_json::job_spec_from_json;
use rastai_diag::logger::log_warn;
use rastai_proto::common::v1::JobState;
use rastai_proto::node::v1::{JobKind, JobSpec};

use crate::jobs::exit_cause::NodeKill;
use crate::jobs::job_file::{JOB_FILE, JobFile, read_job_file, recorded_process};
use crate::jobs::log_lines::{JobLogs, STDERR_LOG_FILE, STDOUT_LOG_FILE, follow_job_logs};
use crate::jobs::signals::{ProcessIdentity, is_running_process};
use crate::jobs::supervisor::{adopt_reattached_job, finish_vanished_job, job_has_ended, job_state, kill_job_group};
use crate::node::NodeState;

/// How long a reattached job's worker has to register again (contracts 15.10; `rastai_worker` retries for as long).
pub const HOST_RECONNECT_TIMEOUT_SECONDS: u64 = 300;
/// How often the node looks at a reattached job's process. It isn't the node's child, so there is no exit to wait
/// on (layout 7.4: "polls `/proc/<pid>` once a second").
const PROCESS_POLL_SECONDS: u64 = 1;
/// `train` → `JOB_KIND_TRAIN`, the enum's own name.
const JOB_KIND_NAME_PREFIX: &str = "JOB_KIND_";

/// One `job.json` without a final state, and the run folder it was found in.
struct UnfinishedJob {
    run_folder: PathBuf,
    job_file: JobFile,
}

/// Reads every run's `job.json` and takes back each job that hasn't ended: a live one waits `lost` for its worker,
/// a gone one ends from its log. Returns once every such job is in the table; the waits and ends go on in the
/// background.
pub async fn reattach_jobs(state: &Arc<NodeState>) {
    let runs_folder = runs_folder(state);
    let found = tokio::task::spawn_blocking(move || find_unfinished_jobs(&runs_folder)).await;
    let unfinished = found.unwrap_or_default();
    for job in unfinished {
        reattach_job(state, job);
    }
}

/// `<data>/runs/`, the folder that holds every run folder: `run_dir` with no run id appended.
fn runs_folder(state: &NodeState) -> PathBuf {
    run_dir(&state.paths, "")
}

/// Every `job.json` under `runs_folder` that has no `final_state` and whose ids are ids. A missing runs folder has
/// none.
fn find_unfinished_jobs(runs_folder: &Path) -> Vec<UnfinishedJob> {
    let Ok(entries) = std::fs::read_dir(runs_folder) else {
        return Vec::new();
    };
    let mut unfinished = Vec::new();
    for entry in entries.flatten() {
        let run_folder = entry.path();
        if let Some(job_file) = unfinished_job_file(&run_folder) {
            unfinished.push(UnfinishedJob { run_folder, job_file });
        }
    }
    unfinished
}

/// The run folder's `job.json`, when it names a job that hasn't ended. A folder that isn't a run's, or holds no
/// `job.json`, has none; a damaged one is logged.
fn unfinished_job_file(run_folder: &Path) -> Option<JobFile> {
    let folder_name = run_folder.file_name()?.to_str()?;
    if !id_has_prefix(folder_name, IdPrefix::Run) {
        return None;
    }
    let path = run_folder.join(JOB_FILE);
    if !path.is_file() {
        return None;
    }
    let job_file = match read_job_file(&path) {
        Ok(job_file) => job_file,
        Err(reason) => {
            log_warn(
                "a job.json couldn't be read, so its job wasn't reattached",
                &[("reason", &reason)],
            );
            return None;
        }
    };
    if !job_file.final_state.is_empty() {
        return None;
    }
    // The ids become socket and folder names, and the job's own run folder is where its job.json must be.
    let ids_fit = id_has_prefix(&job_file.job_id, IdPrefix::Job) && job_file.run_id == folder_name;
    if !ids_fit {
        let shown = path.display().to_string();
        log_warn(
            "a job.json names ids that don't fit its run folder, so its job wasn't reattached",
            &[("path", &shown)],
        );
        return None;
    }
    Some(job_file)
}

/// Takes one job back: `lost` with its watchers when its process is still its own, else ended from its log.
fn reattach_job(state: &Arc<NodeState>, job: UnfinishedJob) {
    let spec = reattached_spec(&job.job_file);
    let process = recorded_process(&job.job_file).filter(is_running_process);
    if !adopt_reattached_job(state, &spec, &job.job_file, process.clone()) {
        return;
    }
    match process {
        Some(identity) => watch_reattached_job(state, &spec.job_id, &job.run_folder, identity),
        None => end_gone_job(state, &spec.job_id),
    }
}

/// The job's spec as `job.json` keeps it. The file's own ids win over the spec's, and a spec that can't be read
/// leaves the job with its ids, kind and environment only, so no permission is ever guessed.
fn reattached_spec(job_file: &JobFile) -> JobSpec {
    let mut spec = match job_spec_from_json(&job_file.job_spec) {
        Ok(spec) => spec,
        Err(_) => JobSpec {
            kind: job_kind_from_name(&job_file.kind) as i32,
            environment_id: job_file.environment_id.clone(),
            ..JobSpec::default()
        },
    };
    spec.job_id = job_file.job_id.clone();
    spec.run_id = job_file.run_id.clone();
    spec
}

/// `train` → `JobKind::Train`; a name this build doesn't know is unspecified.
fn job_kind_from_name(name: &str) -> JobKind {
    let enum_name = format!("{JOB_KIND_NAME_PREFIX}{}", name.to_ascii_uppercase());
    JobKind::from_str_name(&enum_name).unwrap_or(JobKind::Unspecified)
}

/// A gone job's end reads the run's whole log and writes `job.json`: blocking file work, off the runtime's threads.
fn end_gone_job(state: &Arc<NodeState>, job_id: &str) {
    let state = Arc::clone(state);
    let job_id = job_id.to_string();
    tokio::task::spawn_blocking(move || finish_vanished_job(&state, &job_id));
}

/// Starts a live reattached job's three watchers: its process, its time to register again, and its output lines.
fn watch_reattached_job(state: &Arc<NodeState>, job_id: &str, run_folder: &Path, identity: ProcessIdentity) {
    tokio::spawn(follow_job_logs(
        Arc::clone(state),
        job_id.to_string(),
        logs_from_their_ends(run_folder),
    ));
    tokio::spawn(watch_reattached_process(
        Arc::clone(state),
        job_id.to_string(),
        identity,
    ));
    tokio::spawn(wait_for_reconnect(Arc::clone(state), job_id.to_string()));
}

/// The job's logs, followed from their current ends: what it printed while no node was there was for a live
/// listener that wasn't there either (log lines are ephemeral).
fn logs_from_their_ends(run_folder: &Path) -> JobLogs {
    let length = |name: &str| std::fs::metadata(run_folder.join(name)).map_or(0, |metadata| metadata.len());
    JobLogs {
        stdout_path: run_folder.join(STDOUT_LOG_FILE),
        stderr_path: run_folder.join(STDERR_LOG_FILE),
        stdout_offset: length(STDOUT_LOG_FILE),
        stderr_offset: length(STDERR_LOG_FILE),
    }
}

/// Polls the job's process until it is gone, then ends the job from what it left. A job that ended some other way
/// meanwhile needs nothing more.
async fn watch_reattached_process(state: Arc<NodeState>, job_id: String, identity: ProcessIdentity) {
    loop {
        tokio::time::sleep(Duration::from_secs(PROCESS_POLL_SECONDS)).await;
        if job_has_ended(&state, &job_id) {
            return;
        }
        if !is_running_process(&identity) {
            break;
        }
    }
    let finished = tokio::task::spawn_blocking(move || finish_vanished_job(&state, &job_id)).await;
    if let Err(error) = finished {
        log_warn("ending a reattached job failed", &[("reason", &error.to_string())]);
    }
}

/// Kills the job's group when its worker hasn't registered again within the reconnect timeout (`RRS-J3003`). The
/// kill checks the pid's identity first; the process watcher then sees it gone and ends the job.
async fn wait_for_reconnect(state: Arc<NodeState>, job_id: String) {
    tokio::time::sleep(state.job_options.host_reconnect_timeout).await;
    if job_state(&state, &job_id) != Some(JobState::Lost) {
        return;
    }
    if let Err(reason) = kill_job_group(&state, &job_id, NodeKill::ReconnectTimeout).await {
        log_warn(
            "a job that didn't reconnect couldn't be killed",
            &[("job_id", &job_id), ("reason", &reason)],
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2S";
    const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q";

    struct TestFolder {
        path: PathBuf,
    }

    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn test_folder(name: &str) -> TestFolder {
        let path = std::env::temp_dir().join(format!("rastai-reattach-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TestFolder { path }
    }

    fn job_file(final_state: &str) -> JobFile {
        JobFile {
            job_id: JOB_ID.to_string(),
            run_id: RUN_ID.to_string(),
            kind: "train".to_string(),
            final_state: final_state.to_string(),
            ..JobFile::default()
        }
    }

    fn write_run(runs: &Path, folder_name: &str, job_file: &JobFile) {
        let run_folder = runs.join(folder_name);
        std::fs::create_dir_all(&run_folder).unwrap();
        crate::jobs::job_file::write_job_file(&run_folder, job_file).unwrap();
    }

    // Only a run folder's job.json without a final state, whose ids fit the folder, is taken back.
    #[test]
    fn only_unfinished_job_files_of_run_folders_are_found() {
        let folder = test_folder("find");
        write_run(&folder.path, RUN_ID, &job_file(""));
        write_run(&folder.path, "run_01JA2B3C4D5E6F7G8H9J0K1M2T", &job_file(""));
        write_run(
            &folder.path,
            "run_01JA2B3C4D5E6F7G8H9J0K1M2V",
            &JobFile {
                run_id: "run_01JA2B3C4D5E6F7G8H9J0K1M2V".to_string(),
                ..job_file("succeeded")
            },
        );
        write_run(&folder.path, "not-a-run", &job_file(""));
        std::fs::create_dir_all(folder.path.join("run_01JA2B3C4D5E6F7G8H9J0K1M2W")).unwrap();
        let found = find_unfinished_jobs(&folder.path);
        assert_eq!(found.len(), 1, "only the first run's own file");
        assert_eq!(found[0].job_file.job_id, JOB_ID);
        assert_eq!(found[0].run_folder, folder.path.join(RUN_ID));
        assert!(find_unfinished_jobs(&folder.path.join("missing")).is_empty());
    }

    #[test]
    fn the_spec_keeps_the_files_ids_and_falls_back_to_its_kind() {
        let with_spec = JobFile {
            job_spec: json!({ "job_id": "job_other", "kind": "JOB_KIND_EVAL", "network_hosts": ["db.lab"] }),
            ..job_file("")
        };
        let spec = reattached_spec(&with_spec);
        assert_eq!((spec.job_id.as_str(), spec.run_id.as_str()), (JOB_ID, RUN_ID));
        assert_eq!(spec.kind(), JobKind::Eval);
        assert_eq!(spec.network_hosts, vec!["db.lab".to_string()]);
        let without_spec = reattached_spec(&job_file(""));
        assert_eq!(without_spec.kind(), JobKind::Train);
        assert!(without_spec.network_hosts.is_empty());
        assert_eq!(job_kind_from_name("doctor_selftest"), JobKind::DoctorSelftest);
        assert_eq!(job_kind_from_name("no such kind"), JobKind::Unspecified);
    }
}
