//! The in-house JSON-lines logger (spec v5.4 §25, layout: rust-control-plane.md section 3.2). There is no `tracing`.
//!
//! One line per event: `{"level":"info","message":"...","process":"daemon","run_id":"...","unix_ms":...}` plus the
//! caller's fields, in `<state>/logs/daemon.jsonl` or `node.jsonl`. When the file reaches `logging.max_file_bytes` it
//! is rotated to `daemon.1.jsonl`, `daemon.2.jsonl` ..., keeping `logging.keep_files` old files.
//!
//! This is the only file in the crate that does I/O, and only after `init_logger` was given a file. Before that
//! (and always in the WASM build) every log call returns at once.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::diagnostic::{Diagnostic, Severity};

// The keys every line has. A caller field with one of these names is written as `field_<name>`, so it can't
// overwrite them (and a line never has the same key twice, which our own strict JSON reader would refuse).
const KEY_UNIX_MS: &str = "unix_ms";
const KEY_LEVEL: &str = "level";
const KEY_PROCESS: &str = "process";
const KEY_MESSAGE: &str = "message";
const RESERVED_KEYS: [&str; 4] = [KEY_UNIX_MS, KEY_LEVEL, KEY_PROCESS, KEY_MESSAGE];
const RENAMED_FIELD_PREFIX: &str = "field_";

/// `max_file_bytes` of 0 means "never rotate", so a missing setting can't turn every line into its own file.
const NO_SIZE_LIMIT: u64 = 0;

/// The one logger of this process. A `std::sync::Mutex` guards the file, and each line is one `write_all`, so lines
/// from different threads never interleave.
static LOGGER: Mutex<Option<OpenLog>> = Mutex::new(None);

/// Ordered from least to most severe, so a line is written when its level is at least the configured one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// What `init_logger` needs: the `logging` settings section (contracts 11.2) and who is writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggerSettings {
    /// `daemon` or `node`; written into every line.
    pub process_name: String,
    /// The `.jsonl` file. None keeps the logger quiet: lines are dropped.
    pub file_path: Option<PathBuf>,
    pub level: LogLevel,
    pub max_file_bytes: u64,
    /// How many rotated files to keep beside the current one.
    pub keep_files: u32,
}

/// The logger's state once `init_logger` ran.
struct OpenLog {
    settings: LoggerSettings,
    file: Option<File>,
    file_bytes: u64,
}

/// Starts (or restarts) the process's logger. The entry functions of `rastai-server` and `rastai-node` call it once.
/// It creates the log folder if needed. If the file can't be opened, logging is off rather than fatal: a full disk
/// must not stop the daemon from starting.
pub fn init_logger(settings: LoggerSettings) {
    let open_log = open_log(settings);
    *lock_logger() = Some(open_log);
}

pub fn log_info(message: &str, fields: &[(&str, &str)]) {
    write_entry(LogLevel::Info, message, fields);
}

pub fn log_warn(message: &str, fields: &[(&str, &str)]) {
    write_entry(LogLevel::Warn, message, fields);
}

pub fn log_error(message: &str, fields: &[(&str, &str)]) {
    write_entry(LogLevel::Error, message, fields);
}

/// Logs a diagnostic at the level its severity matches, with its code and path as fields.
pub fn log_diagnostic(diagnostic: &Diagnostic, fields: &[(&str, &str)]) {
    let mut all_fields = vec![("code", diagnostic.code.as_str()), ("path", diagnostic.path.as_str())];
    all_fields.extend_from_slice(fields);
    write_entry(
        level_for_severity(diagnostic.severity),
        &diagnostic.message,
        &all_fields,
    );
}

fn level_for_severity(severity: Severity) -> LogLevel {
    match severity {
        Severity::Error => LogLevel::Error,
        Severity::Warning => LogLevel::Warn,
        Severity::Info => LogLevel::Info,
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

/// A thread that panicked while logging leaves the mutex poisoned. Logging carries on: the state is still usable,
/// and losing the logs after a panic would hide the reason for it.
fn lock_logger() -> MutexGuard<'static, Option<OpenLog>> {
    match LOGGER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_entry(level: LogLevel, message: &str, fields: &[(&str, &str)]) {
    let mut guard = lock_logger();
    // Returning before reading the clock matters: `SystemTime::now` panics on wasm32-unknown-unknown.
    let Some(log) = guard.as_mut() else {
        return;
    };
    if log.file.is_none() || level < log.settings.level {
        return;
    }
    let line = format_log_line(current_unix_ms(), level, &log.settings.process_name, message, fields);
    append_line(log, &line);
}

/// One log line, without the newline. Keys come out sorted (serde_json's map), so lines are easy to diff.
fn format_log_line(
    unix_ms: u64,
    level: LogLevel,
    process_name: &str,
    message: &str,
    fields: &[(&str, &str)],
) -> String {
    let mut object = Map::new();
    for (name, value) in fields {
        object.insert(field_key(name), Value::String(value.to_string()));
    }
    object.insert(KEY_UNIX_MS.to_string(), Value::from(unix_ms));
    object.insert(KEY_LEVEL.to_string(), Value::String(level_name(level).to_string()));
    object.insert(KEY_PROCESS.to_string(), Value::String(process_name.to_string()));
    object.insert(KEY_MESSAGE.to_string(), Value::String(message.to_string()));
    Value::Object(object).to_string()
}

fn field_key(name: &str) -> String {
    if RESERVED_KEYS.contains(&name) {
        format!("{RENAMED_FIELD_PREFIX}{name}")
    } else {
        name.to_string()
    }
}

fn current_unix_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        // A clock set before 1970 is the machine's problem; the line still gets written.
        Err(_) => 0,
    }
}

fn open_log(settings: LoggerSettings) -> OpenLog {
    let (file, file_bytes) = match &settings.file_path {
        Some(path) => open_log_file(path),
        None => (None, 0),
    };
    OpenLog {
        settings,
        file,
        file_bytes,
    }
}

/// Opens the file for appending and returns its current size, so rotation counts the lines already there.
fn open_log_file(path: &Path) -> (Option<File>, u64) {
    if let Some(folder) = path.parent()
        && fs::create_dir_all(folder).is_err()
    {
        return (None, 0);
    }
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => {
            let file_bytes = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            (Some(file), file_bytes)
        }
        Err(_) => (None, 0),
    }
}

/// Writes one line, rotating first if it would push the file past the size limit. A failed write drops the line:
/// logging never takes the process down.
fn append_line(log: &mut OpenLog, line: &str) {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    let line_bytes = bytes.len() as u64;

    if needs_rotation(log, line_bytes) {
        rotate(log);
    }
    if let Some(file) = log.file.as_mut()
        && file.write_all(&bytes).is_ok()
    {
        log.file_bytes += line_bytes;
    }
}

/// A file that is still empty is never rotated, so one line longer than the limit still gets written.
fn needs_rotation(log: &OpenLog, line_bytes: u64) -> bool {
    let limit = log.settings.max_file_bytes;
    limit != NO_SIZE_LIMIT && log.file_bytes > 0 && log.file_bytes + line_bytes > limit
}

fn rotate(log: &mut OpenLog) {
    let Some(path) = log.settings.file_path.clone() else {
        return;
    };
    // Close the file before renaming it: Windows can't rename a file that is open.
    log.file = None;
    shift_rotated_files(&path, log.settings.keep_files);
    let (file, file_bytes) = open_log_file(&path);
    log.file = file;
    log.file_bytes = file_bytes;
}

/// `daemon.jsonl` becomes `daemon.1.jsonl`, `daemon.1.jsonl` becomes `daemon.2.jsonl`, and so on; the file that
/// would pass `keep_files` is deleted. Errors are ignored: a rotated file someone deleted by hand is not a problem.
fn shift_rotated_files(path: &Path, keep_files: u32) {
    if keep_files == 0 {
        let _ = fs::remove_file(path);
        return;
    }
    let _ = fs::remove_file(rotated_path(path, keep_files));
    for number in (1..keep_files).rev() {
        let _ = fs::rename(rotated_path(path, number), rotated_path(path, number + 1));
    }
    let _ = fs::rename(path, rotated_path(path, 1));
}

/// `logs/daemon.jsonl` with number 2 is `logs/daemon.2.jsonl`; a file without an extension gets `name.2`.
fn rotated_path(path: &Path, number: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file_name = match path.extension() {
        Some(extension) => format!("{stem}.{number}.{}", extension.to_string_lossy()),
        None => format!("{stem}.{number}"),
    };
    path.with_file_name(file_name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A fresh folder per test under the system temp folder, named after the process so parallel runs don't meet.
    fn scratch_folder(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-diag-{}-{test_name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn settings_for(path: &Path, max_file_bytes: u64, keep_files: u32) -> LoggerSettings {
        LoggerSettings {
            process_name: "daemon".to_string(),
            file_path: Some(path.to_path_buf()),
            level: LogLevel::Info,
            max_file_bytes,
            keep_files,
        }
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        let text = fs::read_to_string(path).unwrap();
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    #[test]
    fn a_line_has_the_fixed_keys_and_the_fields() {
        let line = format_log_line(
            1700,
            LogLevel::Warn,
            "node",
            "said \"hi\"",
            &[("run_id", "run_1"), ("level", "x")],
        );
        assert_eq!(
            line,
            r#"{"field_level":"x","level":"warn","message":"said \"hi\"","process":"node","run_id":"run_1","unix_ms":1700}"#
        );
    }

    #[test]
    fn rotation_keeps_the_newest_files() {
        let folder = scratch_folder("rotation");
        let path = folder.join("logs").join("daemon.jsonl");
        // Each line is 11 bytes with its newline, so a 30-byte limit fits two lines per file.
        let mut log = open_log(settings_for(&path, 30, 2));
        for number in 0..7 {
            append_line(&mut log, &format!("{{\"line\":{number}}}"));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"line\":6}\n");
        assert_eq!(
            fs::read_to_string(folder.join("logs/daemon.1.jsonl")).unwrap(),
            "{\"line\":4}\n{\"line\":5}\n"
        );
        assert_eq!(
            fs::read_to_string(folder.join("logs/daemon.2.jsonl")).unwrap(),
            "{\"line\":2}\n{\"line\":3}\n"
        );
        assert!(!folder.join("logs/daemon.3.jsonl").exists());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn keep_zero_starts_the_file_over_and_limit_zero_never_rotates() {
        let folder = scratch_folder("keep_zero");
        let path = folder.join("node.jsonl");
        let mut log = open_log(settings_for(&path, 15, 0));
        append_line(&mut log, "{\"line\":1}");
        append_line(&mut log, "{\"line\":2}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"line\":2}\n");
        assert!(!folder.join("node.1.jsonl").exists());

        let mut unlimited = open_log(settings_for(&path, NO_SIZE_LIMIT, 3));
        append_line(&mut unlimited, "{\"line\":3}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"line\":2}\n{\"line\":3}\n");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn rotated_names_keep_the_extension_last() {
        assert_eq!(
            rotated_path(Path::new("logs/daemon.jsonl"), 3),
            PathBuf::from("logs/daemon.3.jsonl")
        );
        assert_eq!(
            rotated_path(Path::new("logs/daemon"), 1),
            PathBuf::from("logs/daemon.1")
        );
    }

    // The only test that touches the process-wide logger; others in the crate may log at the same time (an unknown
    // code logs a warning), so it looks only at the lines carrying its own marker.
    #[test]
    fn the_process_logger_filters_by_level_and_logs_diagnostics() {
        let folder = scratch_folder("process_logger");
        let path = folder.join("daemon.jsonl");
        init_logger(settings_for(&path, NO_SIZE_LIMIT, 1));
        let marker = ("test", "process_logger");
        write_entry(LogLevel::Debug, "dropped", &[marker]);
        log_info("started", &[marker, ("run_id", "run_1")]);
        log_error("failed", &[marker]);
        let diagnostic = Diagnostic {
            code: "RRS-S3001".to_string(),
            severity: Severity::Warning,
            message: "disk nearly full".to_string(),
            path: "#/storage".to_string(),
            fix_text: String::new(),
            fix_command_id: String::new(),
            fix_arguments_json: String::new(),
        };
        log_diagnostic(&diagnostic, &[marker]);
        init_logger(LoggerSettings {
            file_path: None,
            ..settings_for(&path, 0, 0)
        });
        log_info("after the logger was turned off", &[marker]);

        let lines: Vec<Value> = read_lines(&path)
            .into_iter()
            .filter(|line| line["test"] == "process_logger")
            .collect();
        let summary: Vec<(String, String)> = lines
            .iter()
            .map(|line| {
                (
                    line["level"].as_str().unwrap().to_string(),
                    line["message"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let expected = [("info", "started"), ("error", "failed"), ("warn", "disk nearly full")]
            .map(|(level, message)| (level.to_string(), message.to_string()));
        assert_eq!(summary, expected);
        assert_eq!(lines[0]["process"], "daemon");
        assert_eq!(lines[0]["run_id"], "run_1");
        assert!(lines[0]["unix_ms"].as_u64().unwrap() > 0);
        assert_eq!(lines[2]["code"], "RRS-S3001");
        assert_eq!(lines[2]["path"], "#/storage");
        fs::remove_dir_all(&folder).unwrap();
    }
}
