//! The launch of one queued job (layout: rust-control-plane.md 8.1; flows/platform.md 5.1, training flow 4.1):
//! `run.json` holds the job's `JobSpec`, written when the job is submitted; the launch reads it back and calls
//! `NodeService.StartJob`, then starts the run's tail at sequence 0, so nothing the job writes is missed.
//!
//! - **`run.json`** (`rastai.run_record/1`, contracts 5.2) is written once, atomically, before the job is queued, so
//!   a queued job survives a restart and a rebuild of `rastai.db` knows what the run was started with. The hardware
//!   fingerprint is only known on the node: the worker reports it in its first record.
//! - **A node that can't be reached** leaves the job queued; the scheduler tries again on its next pass. A start the
//!   node refuses (`RRS-N0004`, `RRS-J3006`) or a spawn that fails comes back as the job's own `failed` state.
//! - **A job that can't be launched at all** (its `run.json` is missing or damaged) fails with the reason, rather
//!   than staying queued forever.

// Every function here answers a catalog `Diagnostic` on failure, which is larger than clippy's limit for an error
// type; boxing it would only add indirection to code the owner reads.
#![allow(clippy::result_large_err)]

use std::path::Path;
use std::sync::Arc;

use rastai_core::paths::{DataPaths, run_dir};
use rastai_core::proto_json::{job_spec_from_json, job_spec_to_json};
use rastai_core::states::{is_terminal_job_state, job_state_name};
use rastai_core::time::{format_rfc3339, now_unix_ms};
use rastai_core::versions::APP_VERSION;
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::Document;
use rastai_formats::documents::run_record::{RUN_RECORD_TYPE_NAME, RunKind, RunRecipe, RunRecord};
use rastai_formats::header::SharedHeader;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{read_document, write_document};
use rastai_proto::common::v1::JobState;
use rastai_proto::node::v1::{JobInfo, JobKind, JobSpec};
use rastai_store::config_files::write_config_file;
use rastai_store::index::runs::JobRow;

use crate::error::store_error_diagnostic;
use crate::nodes::node_links::start_job_on_node;
use crate::nodes::reconcile::{
    SnapshotScope, fail_unlaunchable_job, read_job, record_job_changes, settle_runs_later, tail_then_settle,
};
use crate::server_state::Server;

/// `runs/<run_id>/run.json` (contracts 8.4).
const RUN_RECORD_FILE: &str = "run.json";
/// The schema URL of a new run record (contracts 5.1). The domain is still open (contracts 14 row 8); the generated
/// schemas use this one.
const RUN_RECORD_SCHEMA_URL: &str = "https://schemas.rastai.dev/rastai.run_record/1.json";
const RUN_RECORD_SCHEMA_VERSION: u32 = 1;
/// `RRS-C0002` for a `run.json` that can't be read back as a run record with a `JobSpec`.
const CODE_INVALID_DOCUMENT: &str = "RRS-C0002";
const EXPECTED_RUN_RECORD: &str = "a rastai.run_record/1 run.json with the job's job_spec";
/// The proto names of job kinds start with this; contracts 9.1 names them without it, in lower case.
const JOB_KIND_NAME_PREFIX: &str = "JOB_KIND_";

/// What a new run starts with, beyond its job's `JobSpec`: what `run.json` and its `runs` row need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    /// The project the run belongs to; empty outside one.
    pub project_id: String,
    pub kind: RunKind,
    /// The recipe's id (`classification` in M1).
    pub recipe: String,
    /// The run's one seed, as `TrainJobArgs.seed` carries it.
    pub seed: Option<u64>,
}

impl RunStart {
    /// The run kind's name in contracts 9.5, as `runs.kind` and `run.json` hold it (`train`).
    pub fn kind_name(&self) -> String {
        match serde_json::to_value(self.kind) {
            Ok(serde_json::Value::String(name)) => name,
            _ => String::new(),
        }
    }
}

/// Writes the run's `run.json` with `spec` (its `job_id` filled in) and returns the record's content hash, which the
/// `runs` row keeps as `record_hash`.
pub fn write_run_record(paths: &DataPaths, spec: &JobSpec, run: &RunStart) -> Result<String, Diagnostic> {
    let record = new_run_record(spec, run);
    let document = Document::RunRecord(record);
    let bytes = write_document(&document).map_err(|error| invalid_run_record(&format!("{error:?}")))?;
    let path = run_dir(paths, &spec.run_id).join(RUN_RECORD_FILE);
    write_config_file(&path, &bytes).map_err(|error| store_error_diagnostic(&error))?;
    match read_run_record(&path)? {
        Some(record) => Ok(record.header.content_hash),
        None => Err(invalid_run_record("the file just written doesn't read back")),
    }
}

/// The `JobSpec` of a run's `run.json`.
pub fn read_run_job_spec(paths: &DataPaths, run_id: &str) -> Result<JobSpec, Diagnostic> {
    let path = run_dir(paths, run_id).join(RUN_RECORD_FILE);
    let Some(record) = read_run_record(&path)? else {
        return Err(invalid_run_record("it isn't a run record"));
    };
    let Some(spec_json) = record.job_spec else {
        return Err(invalid_run_record("it has no job_spec"));
    };
    job_spec_from_json(&spec_json).map_err(|reason| invalid_run_record(&reason))
}

/// Starts one queued job on its node and begins tailing its run. See the top for what each failure does.
///
/// `Scheduler.launch_lock` is held from the queue check to the recorded answer, so a `job.cancel` of the queued job
/// either comes first (and the job is never started) or finds it started (and signals the node). The job lock is
/// taken only to check and to record, never across `StartJob`, which may take up to the node call timeout: a
/// heartbeat waits for the job lock, and the node's hub client gives up long before that.
pub async fn launch_job(server: &Arc<Server>, job: &JobRow) {
    let _launching = server.scheduler.launch_lock.lock().await;
    if !is_still_queued(server, &job.job_id).await {
        return;
    }
    let spec = match read_run_job_spec(&server.paths, &job.run_id) {
        Ok(spec) if spec.job_id == job.job_id => spec,
        Ok(_) => {
            let problem = invalid_run_record("its job_spec is another job's");
            fail_unlaunchable_job(server, job, problem).await;
            return;
        }
        Err(problem) => {
            fail_unlaunchable_job(server, job, problem).await;
            return;
        }
    };
    let info = match start_job_on_node(server, &job.node_id, spec).await {
        Ok(info) => info,
        Err(problem) => {
            log_warn(
                "a queued job couldn't reach its node; it stays queued",
                &[
                    ("job_id", &job.job_id),
                    ("code", &problem.code),
                    ("message", &problem.message),
                ],
            );
            return;
        }
    };
    let ended_runs = {
        let _held = server.scheduler.job_state_lock.lock().await;
        record_start(server, job, info).await
    };
    settle_runs_later(server, ended_runs);
}

/// The tail starts before the job's state is recorded, so a settle the new state asks for waits for the tail to
/// read the log to its end (nodes/reconcile.rs). Returns the run when the start already ended the job. The caller
/// holds the job lock.
///
/// A heartbeat that arrived during `StartJob` may have recorded the job's state already; what it brought is at
/// least as new as this answer, so the answer is then not recorded, which keeps a job from moving back.
async fn record_start(server: &Arc<Server>, job: &JobRow, info: JobInfo) -> Vec<String> {
    if !is_terminal_job_state(info.state()) {
        tail_then_settle(server, &job.node_id, &job.run_id);
    }
    if !job_row_is_queued(server, &job.job_id) {
        return Vec::new();
    }
    record_job_changes(server, &job.node_id, &[info], SnapshotScope::TheseJobs).await
}

/// Whether the job is still queued, read under the job lock.
async fn is_still_queued(server: &Server, job_id: &str) -> bool {
    let _held = server.scheduler.job_state_lock.lock().await;
    job_row_is_queued(server, job_id)
}

fn job_row_is_queued(server: &Server, job_id: &str) -> bool {
    let queued = job_state_name(JobState::Queued);
    read_job(server, job_id).is_some_and(|row| row.state == queued)
}

/// A job kind's name in contracts 9.1 (`JOB_KIND_TRAIN` → `train`), as `jobs.kind` holds it.
pub fn job_kind_name(kind: i32) -> String {
    match JobKind::try_from(kind) {
        Ok(kind) => kind
            .as_str_name()
            .trim_start_matches(JOB_KIND_NAME_PREFIX)
            .to_ascii_lowercase(),
        Err(_) => kind.to_string(),
    }
}

fn new_run_record(spec: &JobSpec, run: &RunStart) -> RunRecord {
    RunRecord {
        header: SharedHeader {
            schema_url: RUN_RECORD_SCHEMA_URL.to_string(),
            document_type: RUN_RECORD_TYPE_NAME.to_string(),
            schema_version: RUN_RECORD_SCHEMA_VERSION,
            created_with: APP_VERSION.to_string(),
            id: spec.run_id.clone(),
            content_hash: String::new(),
            derived_from: Vec::new(),
            authors: Vec::new(),
            license: String::new(),
            source_url: String::new(),
            requires: Default::default(),
        },
        run_id: spec.run_id.clone(),
        kind: run.kind,
        job_spec: Some(job_spec_to_json(spec)),
        recipe: Some(RunRecipe {
            name: run.recipe.clone(),
            settings: Default::default(),
        }),
        dataset_hashes: Vec::new(),
        base_model: None,
        plugin_versions: Default::default(),
        environment_lock_hash: String::new(),
        seed: run.seed,
        hardware_fingerprint: None,
        git_commit: String::new(),
        app_version: APP_VERSION.to_string(),
        parent_run_id: String::new(),
        chain: None,
        source: None,
        created_at: format_rfc3339(now_unix_ms()),
    }
}

/// The run record in `path` through rastai-formats' four layers; None when the file holds another document type.
fn read_run_record(path: &Path) -> Result<Option<RunRecord>, Diagnostic> {
    let bytes = std::fs::read(path).map_err(|error| invalid_run_record(&error.to_string()))?;
    let file_name = path.display().to_string();
    let outcome = read_document(&file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
    if let Some(problem) = outcome
        .diagnostics
        .into_iter()
        .find(|item| item.severity == Severity::Error)
    {
        return Err(problem);
    }
    match outcome.document {
        Some(Document::RunRecord(record)) => Ok(Some(record)),
        _ => Ok(None),
    }
}

fn invalid_run_record(found: &str) -> Diagnostic {
    diagnostic(
        CODE_INVALID_DOCUMENT,
        &[("found", found), ("expected", EXPECTED_RUN_RECORD)],
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_proto::node::v1::JobKind;

    fn temporary_paths(name: &str) -> DataPaths {
        let root = std::env::temp_dir().join(format!("rsv-launch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        }
    }

    fn train_run() -> RunStart {
        RunStart {
            project_id: String::new(),
            kind: RunKind::Train,
            recipe: "classification".to_string(),
            seed: Some(7),
        }
    }

    // A queued job's spec survives in run.json: what the launch reads back is what was submitted.
    #[test]
    fn the_job_spec_reads_back_from_run_json() {
        let paths = temporary_paths("roundtrip");
        let spec = JobSpec {
            job_id: "job_01JA2B3C4D5E6F7G8H9J0K1M2N".to_string(),
            run_id: "run_01JA2B3C4D5E6F7G8H9J0K1M2P".to_string(),
            kind: JobKind::Train as i32,
            environment_id: "cpu".to_string(),
            arguments_json: r#"{"max_steps":40}"#.to_string(),
            data_policy: "local-only".to_string(),
            ..JobSpec::default()
        };
        let record_hash = write_run_record(&paths, &spec, &train_run()).unwrap();
        assert!(record_hash.starts_with("blake3:"), "{record_hash}");
        assert_eq!(read_run_job_spec(&paths, &spec.run_id).unwrap(), spec);
        let _ = std::fs::remove_dir_all(paths.data.parent().unwrap());
    }

    #[test]
    fn a_missing_run_json_is_a_coded_problem() {
        let paths = temporary_paths("missing");
        let problem = read_run_job_spec(&paths, "run_01JA2B3C4D5E6F7G8H9J0K1M2P").unwrap_err();
        assert_eq!(problem.code, CODE_INVALID_DOCUMENT);
    }

    #[test]
    fn names_are_the_contract_ones() {
        assert_eq!(job_kind_name(JobKind::Train as i32), "train");
        assert_eq!(job_kind_name(JobKind::DoctorSelftest as i32), "doctor_selftest");
        assert_eq!(job_kind_name(999), "999");
        assert_eq!(train_run().kind_name(), "train");
    }
}
