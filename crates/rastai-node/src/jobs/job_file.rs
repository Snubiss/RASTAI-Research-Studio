//! `runs/<run_id>/job.json`: what the worker reads at start and what a restarted node reattaches from (contracts
//! 8.4; layout: rust-node-gateway-proto.md section 7.4 step 3; python layout 7.3).
//!
//! The run's one live job owns the file (contracts 9.1, "One live job per run"): the node writes it only after the
//! job was admitted and the run reserved for it, rewrites it with the process's identity after the spawn, and a
//! last time with `final_state` when the job ends. Every write is a whole new file put in place by rename, mode
//! 0600 (a `serve_fast` job's file will hold llama-server's key), so a reader never sees half a file.
//!
//! Fields this task can't fill yet are written empty, so the worker always finds them: `environment_lock_hash`
//! (uv environments, M2-16), `granted_leases` (VRAM leases, M3), `resolved_models` (models, M3), `plugins` (M4)
//! and `resume_from` (`train.resume`). `cache_folder` has no path in contracts 8.1 yet and joins with its first
//! user (anchors and tokenized datasets, M5).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The file's name in the run folder.
pub const JOB_FILE: &str = "job.json";
/// The name the next version is written under before the rename.
const JOB_FILE_TEMPORARY: &str = "job.json.tmp";
/// Only the user may read it: a `serve_fast` job's file holds llama-server's key (contracts 8.4).
#[cfg(unix)]
const JOB_FILE_MODE: u32 = 0o600;

/// Everything the node writes into `job.json`, as plain values. `job_spec` and `hardware_profile` are already in
/// the protobuf JSON mapping (rastai-core's proto_json.rs), which the worker decodes with `json_format`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobFile {
    pub job_id: String,
    pub run_id: String,
    /// The job kind's name in contracts 9.1 (`train`, `doctor_selftest`).
    pub kind: String,
    /// The job state's name (`job_state_name`).
    pub state: String,
    pub started_unix_ms: i64,
    pub job_spec: Value,
    pub job_socket_path: String,
    pub worker_host_socket_path: String,
    pub run_folder: String,
    pub environment_id: String,
    pub environment_lock_hash: String,
    pub hardware_profile: Value,
    pub resume_from: String,
    pub pid: u32,
    pub process_group_id: u32,
    pub process_start_ticks: u64,
    pub boot_id: String,
    /// `blake3:<64 hex>` of the job token's 64 hex characters (contracts 4.5); the token itself is never on disk.
    pub job_token_hash: String,
    /// Empty while the job is live, then `succeeded`, `failed` or `cancelled`.
    pub final_state: String,
}

/// `<run folder>/job.json`.
pub fn job_file_path(run_folder: &Path) -> PathBuf {
    run_folder.join(JOB_FILE)
}

/// Writes `job.json` into `run_folder` as a whole: a temporary file (mode 0600, synced to disk), renamed over the
/// old one, then the folder synced so the rename itself survives a crash. The error says what failed.
pub fn write_job_file(run_folder: &Path, job_file: &JobFile) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(&job_file_json(job_file))
        .map_err(|error| format!("{JOB_FILE} can't be encoded: {error}"))?;
    text.push('\n');
    let temporary_path = run_folder.join(JOB_FILE_TEMPORARY);
    write_private_file(&temporary_path, text.as_bytes())?;
    if let Err(error) = fs::rename(&temporary_path, job_file_path(run_folder)) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("{JOB_FILE} can't be put in place: {error}"));
    }
    sync_folder(run_folder);
    Ok(())
}

/// Reads `job.json` back. A field that is missing or of another type reads as empty, so a file from an older node
/// still reads; a file that isn't a JSON object is an error.
pub fn read_job_file(path: &Path) -> Result<JobFile, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{} can't be read: {error}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("{} isn't JSON: {error}", path.display()))?;
    let Some(object) = value.as_object() else {
        return Err(format!("{} isn't a JSON object", path.display()));
    };
    Ok(job_file_from_object(object))
}

fn job_file_json(job_file: &JobFile) -> Value {
    json!({
        "job_id": job_file.job_id,
        "run_id": job_file.run_id,
        "kind": job_file.kind,
        "state": job_file.state,
        "started_unix_ms": job_file.started_unix_ms,
        "job_spec": job_file.job_spec,
        "job_socket_path": job_file.job_socket_path,
        "worker_host_socket_path": job_file.worker_host_socket_path,
        "run_folder": job_file.run_folder,
        "environment_id": job_file.environment_id,
        "environment_lock_hash": job_file.environment_lock_hash,
        "hardware_profile": job_file.hardware_profile,
        "granted_leases": [],
        "resolved_models": {},
        "plugins": [],
        "resume_from": job_file.resume_from,
        "pid": job_file.pid,
        "process_group_id": job_file.process_group_id,
        "process_start_ticks": job_file.process_start_ticks,
        "boot_id": job_file.boot_id,
        "job_token_hash": job_file.job_token_hash,
        "final_state": job_file.final_state,
    })
}

fn job_file_from_object(object: &Map<String, Value>) -> JobFile {
    let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let number = |key: &str| object.get(key).and_then(Value::as_u64).unwrap_or_default();
    let object_value = |key: &str| object.get(key).cloned().unwrap_or_else(|| json!({}));
    JobFile {
        job_id: text("job_id"),
        run_id: text("run_id"),
        kind: text("kind"),
        state: text("state"),
        started_unix_ms: object
            .get("started_unix_ms")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        job_spec: object_value("job_spec"),
        job_socket_path: text("job_socket_path"),
        worker_host_socket_path: text("worker_host_socket_path"),
        run_folder: text("run_folder"),
        environment_id: text("environment_id"),
        environment_lock_hash: text("environment_lock_hash"),
        hardware_profile: object_value("hardware_profile"),
        resume_from: text("resume_from"),
        pid: u32::try_from(number("pid")).unwrap_or_default(),
        process_group_id: u32::try_from(number("process_group_id")).unwrap_or_default(),
        process_start_ticks: number("process_start_ticks"),
        boot_id: text("boot_id"),
        job_token_hash: text("job_token_hash"),
        final_state: text("final_state"),
    }
}

/// Creates or truncates `path` with mode 0600 and writes `bytes` through to the disk.
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = open_private_file(path).map_err(|error| format!("{} can't be created: {error}", path.display()))?;
    // A file left by an earlier crash keeps its old mode when reopened, so the mode is set again.
    set_private_mode(path)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("{} can't be written: {error}", path.display()))
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(JOB_FILE_MODE)
        .open(path)
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create(true).truncate(true).open(path)
}

#[cfg(unix)]
fn set_private_mode(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(JOB_FILE_MODE))
        .map_err(|error| format!("{} can't be made private: {error}", path.display()))
}

/// Windows file access is set by ACLs, which the Windows build handles when it lands (tier 2).
#[cfg(not(unix))]
fn set_private_mode(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Syncs the folder entry that the rename changed. Best effort: a folder can't be opened for syncing everywhere.
fn sync_folder(folder: &Path) {
    if let Ok(handle) = File::open(folder) {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct TestFolder {
        path: PathBuf,
    }

    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn test_folder(name: &str) -> TestFolder {
        let path = std::env::temp_dir().join(format!("rastai-job-file-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestFolder { path }
    }

    fn sample() -> JobFile {
        JobFile {
            job_id: "job_a".to_string(),
            run_id: "run_a".to_string(),
            kind: "train".to_string(),
            state: "starting".to_string(),
            started_unix_ms: 1_791_700_000_123,
            job_spec: json!({ "job_id": "job_a", "kind": "JOB_KIND_TRAIN" }),
            pid: 4242,
            process_group_id: 4242,
            process_start_ticks: 987_654,
            boot_id: "boot".to_string(),
            job_token_hash: "blake3:00".to_string(),
            ..JobFile::default()
        }
    }

    #[test]
    fn a_written_file_reads_back_the_same() {
        let folder = test_folder("round-trip");
        write_job_file(&folder.path, &sample()).unwrap();
        let read = read_job_file(&job_file_path(&folder.path)).unwrap();
        assert_eq!(read, sample());
        assert!(!folder.path.join(JOB_FILE_TEMPORARY).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_even_over_a_leftover_temporary_file() {
        use std::os::unix::fs::PermissionsExt;

        let folder = test_folder("mode");
        let leftover = folder.path.join(JOB_FILE_TEMPORARY);
        fs::write(&leftover, "half a file").unwrap();
        fs::set_permissions(&leftover, fs::Permissions::from_mode(0o644)).unwrap();
        write_job_file(&folder.path, &sample()).unwrap();
        let metadata = fs::metadata(job_file_path(&folder.path)).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, JOB_FILE_MODE);
    }

    #[test]
    fn a_damaged_file_is_an_error_and_missing_fields_read_as_empty() {
        let folder = test_folder("damaged");
        let path = job_file_path(&folder.path);
        fs::write(&path, "[1, 2]").unwrap();
        assert!(read_job_file(&path).is_err());
        fs::write(&path, "{\"job_id\": \"job_b\", \"pid\": \"not a number\"}").unwrap();
        let read = read_job_file(&path).unwrap();
        assert_eq!(read.job_id, "job_b");
        assert_eq!(read.pid, 0);
        assert_eq!(read.final_state, "");
    }
}
