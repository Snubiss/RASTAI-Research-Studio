//! A job's `stdout.log` and `stderr.log` as lines (layout: rust-node-gateway-proto.md section 7.4). The worker's
//! output goes to files, not pipes, so a dead node sends no SIGPIPE and a full pipe can't block the job; the node
//! reads the files instead.
//!
//! - `read_log_tail`: the last lines a job wrote to a log, which a diagnostic about the job's end points at (exit
//!   causes). The logs are appended to across relaunches of a run, so it reads only after the length the log had
//!   when the job was spawned: a job that wrote nothing never shows the previous job's last words as its own.
//! - `read_new_log_lines` and `log_line_event`: the complete lines written since an offset, as `job.log_line`
//!   events (ephemeral, contracts 7.3).
//! - `follow_job_logs`: a running job's two logs, read every `TAIL_POLL_MS` from where its own output starts, each new
//!   line put on the job's live queue for `WatchLive` (live.rs). After the job has ended it reads until nothing is
//!   left, so the last lines go out too, then closes the job's live queue.
//!
//! The files are the job's output, so they are read as untrusted bytes: invalid UTF-8 is replaced, not refused, and
//! a line longer than `MAX_LOG_LINE_BYTES` is cut into pieces of that size, so one huge line can't hold the reader.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rastai_core::time::now_unix_ms;
use rastai_proto::events::v1::{JobLogLine, LogStream};
use rastai_proto::node::v1::LiveEvent;
use rastai_proto::node::v1::live_event::Payload;

use crate::jobs::supervisor::job_has_ended;
use crate::live::{LiveHub, close_live_subject, publish_live_event};
use crate::node::NodeState;
use crate::run_events::TAIL_POLL_MS;

/// The file names in the run folder (contracts 8.4).
pub const STDOUT_LOG_FILE: &str = "stdout.log";
pub const STDERR_LOG_FILE: &str = "stderr.log";
/// How much of the end of a log `read_log_tail` reads. A traceback's last lines fit easily; a log of any size
/// costs the same.
const TAIL_BYTES: u64 = 8 * 1024;
/// The longest line sent as one event. Longer ones are split; a progress bar redrawn with `\r` and no newline is
/// the usual case.
const MAX_LOG_LINE_BYTES: usize = 16 * 1024;
/// The most bytes one `read_new_log_lines` call reads, so a job that floods its output is followed in steps.
const MAX_READ_BYTES: u64 = 1024 * 1024;

/// The complete lines written since `offset`, and where the next read starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewLogLines {
    pub lines: Vec<String>,
    pub next_offset: u64,
}

/// A job's two logs and where the follower reads next in each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JobLogs {
    pub(crate) stdout_path: PathBuf,
    pub(crate) stderr_path: PathBuf,
    pub(crate) stdout_offset: u64,
    pub(crate) stderr_offset: u64,
}

/// Follows a job's logs onto its live queue until the job has ended and every line it wrote has been put there; then
/// closes the queue. Whether the job has ended is read before each read, so the read after the end gets its last
/// lines.
pub(crate) async fn follow_job_logs(state: Arc<NodeState>, job_id: String, logs: JobLogs) {
    let mut logs = logs;
    loop {
        let ended = job_has_ended(&state, &job_id);
        let before = (logs.stdout_offset, logs.stderr_offset);
        let Some(read) = read_job_logs_in_background(logs).await else {
            break;
        };
        logs = read.logs;
        publish_lines(&state.live, &job_id, LogStream::Stdout, read.stdout_lines);
        publish_lines(&state.live, &job_id, LogStream::Stderr, read.stderr_lines);
        let read_nothing = before == (logs.stdout_offset, logs.stderr_offset);
        if ended && read_nothing {
            break;
        }
        if !ended {
            tokio::time::sleep(Duration::from_millis(TAIL_POLL_MS)).await;
        }
    }
    close_live_subject(&state.live, &job_id);
}

/// What one read of both logs found, and where the next one starts.
struct JobLogsRead {
    logs: JobLogs,
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
}

/// Reads the new lines of both logs on a blocking thread. None when that thread failed.
async fn read_job_logs_in_background(logs: JobLogs) -> Option<JobLogsRead> {
    let joined = tokio::task::spawn_blocking(move || {
        let stdout = read_new_log_lines(&logs.stdout_path, logs.stdout_offset);
        let stderr = read_new_log_lines(&logs.stderr_path, logs.stderr_offset);
        JobLogsRead {
            logs: JobLogs {
                stdout_offset: stdout.next_offset,
                stderr_offset: stderr.next_offset,
                ..logs
            },
            stdout_lines: stdout.lines,
            stderr_lines: stderr.lines,
        }
    })
    .await;
    joined.ok()
}

fn publish_lines(hub: &LiveHub, job_id: &str, stream: LogStream, lines: Vec<String>) {
    for line in lines {
        publish_live_event(hub, log_line_event(job_id, stream, line));
    }
}

/// The last `max_lines` lines of the log at `path` that come after `job_start_offset` (the log's length when the
/// job was spawned), oldest first. A missing or unreadable log has none.
pub fn read_log_tail(path: &Path, job_start_offset: u64, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let start = length.saturating_sub(TAIL_BYTES).max(job_start_offset);
    let mut bytes = Vec::new();
    let read = file
        .seek(SeekFrom::Start(start))
        .and_then(|_| file.by_ref().take(TAIL_BYTES).read_to_end(&mut bytes));
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // Reading from the middle of the job's output starts inside a line, which would show as a cut-off first line.
    // At the job's own first byte a line begins, even when the previous job's last line had no newline.
    if start > job_start_offset && !lines.is_empty() {
        lines.remove(0);
    }
    let skip = lines.len().saturating_sub(max_lines);
    lines.split_off(skip)
}

/// Reads the complete lines written to the log at `path` after `offset`. A last line without its newline is left
/// for the next call, unless it is already `MAX_LOG_LINE_BYTES` long. A log that is gone or shorter than `offset`
/// gives no lines and keeps the offset.
pub fn read_new_log_lines(path: &Path, offset: u64) -> NewLogLines {
    let unchanged = NewLogLines {
        lines: Vec::new(),
        next_offset: offset,
    };
    let Ok(mut file) = File::open(path) else {
        return unchanged;
    };
    let mut bytes = Vec::new();
    let read = file
        .seek(SeekFrom::Start(offset))
        .and_then(|_| file.by_ref().take(MAX_READ_BYTES).read_to_end(&mut bytes));
    if read.is_err() {
        return unchanged;
    }
    let (lines, used_bytes) = split_complete_lines(&bytes);
    NewLogLines {
        lines,
        next_offset: offset + used_bytes as u64,
    }
}

/// A `job.log_line` for WatchLive.
pub fn log_line_event(job_id: &str, stream: LogStream, text: String) -> LiveEvent {
    LiveEvent {
        job_id: job_id.to_string(),
        session_id: String::new(),
        emitted_unix_ms: now_unix_ms(),
        payload: Some(Payload::JobLogLine(JobLogLine {
            stream: stream as i32,
            text,
        })),
    }
}

/// The lines in `bytes` that are complete (ended by `\n`, or as long as a line may be), without their endings, and
/// how many bytes they used.
fn split_complete_lines(bytes: &[u8]) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let rest = &bytes[start..];
        let newline = rest.iter().position(|byte| *byte == b'\n');
        let (line_end, used) = match newline {
            Some(position) if position <= MAX_LOG_LINE_BYTES => (position, position + 1),
            _ if rest.len() >= MAX_LOG_LINE_BYTES => (MAX_LOG_LINE_BYTES, MAX_LOG_LINE_BYTES),
            _ => break,
        };
        lines.push(line_text(&rest[..line_end]));
        start += used;
    }
    (lines, start)
}

/// One line's text, without a Windows `\r`, with invalid UTF-8 replaced.
fn line_text(bytes: &[u8]) -> String {
    let without_return = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(without_return).into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::live::{live_subject_ids, take_live_event};

    struct TestFile {
        path: std::path::PathBuf,
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// A folder removed when the test ends, also when an assertion fails.
    struct TestFolder {
        path: std::path::PathBuf,
    }

    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn test_file(name: &str, bytes: &[u8]) -> TestFile {
        let path = std::env::temp_dir().join(format!("rastai-log-lines-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        TestFile { path }
    }

    #[test]
    fn the_tail_is_the_last_lines_in_order() {
        let file = test_file("tail", b"one\ntwo\nthree\nfour\n");
        assert_eq!(read_log_tail(&file.path, 0, 2), vec!["three", "four"]);
        assert_eq!(read_log_tail(&file.path, 0, 10).len(), 4);
        assert!(read_log_tail(&file.path.with_extension("missing"), 0, 3).is_empty());
    }

    // A relaunch appends to the run's log: the tail is only what this job wrote, and nothing when it wrote nothing.
    #[test]
    fn the_tail_holds_only_what_the_job_wrote() {
        let earlier_job = b"the earlier job's last words\nand its partial line";
        let file = test_file("relaunch", earlier_job);
        let job_start_offset = earlier_job.len() as u64;
        assert!(read_log_tail(&file.path, job_start_offset, 20).is_empty());
        let mut both = earlier_job.to_vec();
        both.extend_from_slice(b"this job's line\n");
        std::fs::write(&file.path, &both).unwrap();
        assert_eq!(read_log_tail(&file.path, job_start_offset, 20), vec!["this job's line"]);
        // An offset beyond the log (it was cut since) reads nothing rather than another job's bytes.
        assert!(read_log_tail(&file.path, both.len() as u64 + 10, 20).is_empty());
    }

    // The tail of a long log starts after the cut-off line, never with half of one.
    #[test]
    fn a_long_log_tail_starts_at_a_whole_line() {
        let mut bytes = Vec::new();
        for number in 0..2000 {
            bytes.extend_from_slice(format!("line number {number}\n").as_bytes());
        }
        let file = test_file("long", &bytes);
        let tail = read_log_tail(&file.path, 0, 10_000);
        assert!(tail.first().is_some_and(|line| line.starts_with("line number ")));
        assert_eq!(tail.last().map(String::as_str), Some("line number 1999"));
    }

    #[test]
    fn new_lines_are_read_whole_and_a_partial_line_waits() {
        let file = test_file("new", b"first\r\nsecond\npart");
        let read = read_new_log_lines(&file.path, 0);
        assert_eq!(read.lines, vec!["first", "second"]);
        assert_eq!(read.next_offset, 14);
        let again = read_new_log_lines(&file.path, read.next_offset);
        assert!(again.lines.is_empty());
        assert_eq!(again.next_offset, 14);
    }

    #[test]
    fn a_line_longer_than_the_limit_is_cut_and_bad_bytes_are_replaced() {
        let mut bytes = vec![b'x'; MAX_LOG_LINE_BYTES + 10];
        bytes.extend_from_slice(b"\n\xff\xfeok\n");
        let file = test_file("long-line", &bytes);
        let read = read_new_log_lines(&file.path, 0);
        assert_eq!(read.lines.len(), 3);
        assert_eq!(read.lines[0].len(), MAX_LOG_LINE_BYTES);
        assert_eq!(read.lines[1], "x".repeat(10));
        assert!(read.lines[2].ends_with("ok"));
        assert_eq!(read.next_offset, bytes.len() as u64);
    }

    /// A node state whose folders live under `folder`, with no hardware and no environments.
    fn test_state(folder: &Path) -> NodeState {
        use rastai_core::paths::DataPaths;
        use rastai_core::settings::{default_settings, node_settings, resolve_settings};
        use rastai_proto::common::v1::HardwareProfile;

        use crate::hardware::system_hardware_sources;
        use crate::identity::load_or_create_identity;
        use crate::jobs::supervisor::default_job_options;
        use crate::node::{NodeMode, NodeOptions, build_node_state};

        let options = NodeOptions {
            mode: NodeMode::Embedded,
            paths: DataPaths {
                config: folder.join("config"),
                data: folder.join("data"),
                cache: folder.join("cache"),
                state: folder.join("state"),
                runtime: folder.join("run"),
            },
            settings: node_settings(&resolve_settings(&[default_settings()])),
            hardware_sources: system_hardware_sources(),
            jobs: default_job_options(Vec::new()),
        };
        let identity = load_or_create_identity(&folder.join("node.key")).unwrap();
        build_node_state(options, identity, HardwareProfile::default())
    }

    /// The stream and text of every `job.log_line` waiting on the hub, in the order a stream takes them.
    fn take_log_lines(hub: &LiveHub) -> Vec<(i32, String)> {
        let mut lines = Vec::new();
        while let Some(event) = take_live_event(hub) {
            if let Some(Payload::JobLogLine(line)) = event.payload {
                lines.push((line.stream, line.text));
            }
        }
        lines
    }

    // A job that has ended still has its last lines sent (a crash's traceback is usually the last thing it wrote),
    // and only then is its queue closed. The table doesn't hold this job, so it has ended from the first look.
    #[tokio::test]
    async fn an_ended_job_has_its_last_lines_sent_then_its_queue_closed() {
        let stdout = test_file("ended-stdout", b"step 1\nstep 2\n");
        let stderr = test_file(
            "ended-stderr",
            b"Traceback (most recent call last):\nValueError: boom\n",
        );
        let folder = TestFolder {
            path: std::env::temp_dir().join(format!("rastai-log-lines-{}-ended-state", std::process::id())),
        };
        std::fs::create_dir_all(&folder.path).unwrap();
        let state = Arc::new(test_state(&folder.path));
        let logs = JobLogs {
            stdout_path: stdout.path.clone(),
            stderr_path: stderr.path.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        };
        let followed = follow_job_logs(Arc::clone(&state), "job_ended".to_string(), logs);
        tokio::time::timeout(Duration::from_secs(10), followed).await.unwrap();
        // The queue is closed while its last lines wait, so a later event for the job is dropped.
        publish_live_event(
            &state.live,
            log_line_event("job_ended", LogStream::Stdout, "late".to_string()),
        );
        let expected = vec![
            (LogStream::Stdout as i32, "step 1".to_string()),
            (LogStream::Stdout as i32, "step 2".to_string()),
            (
                LogStream::Stderr as i32,
                "Traceback (most recent call last):".to_string(),
            ),
            (LogStream::Stderr as i32, "ValueError: boom".to_string()),
        ];
        assert_eq!(take_log_lines(&state.live), expected);
        assert!(live_subject_ids(&state.live).is_empty());
    }

    #[test]
    fn a_log_line_event_carries_its_stream_and_text() {
        let event = log_line_event("job_x", LogStream::Stderr, "boom".to_string());
        assert_eq!(event.job_id, "job_x");
        let expected = Payload::JobLogLine(JobLogLine {
            stream: LogStream::Stderr as i32,
            text: "boom".to_string(),
        });
        assert_eq!(event.payload, Some(expected));
    }
}
