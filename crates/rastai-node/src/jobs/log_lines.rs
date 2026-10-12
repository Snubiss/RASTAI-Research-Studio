//! A job's `stdout.log` and `stderr.log` as lines (layout: rust-node-gateway-proto.md section 7.4). The worker's
//! output goes to files, not pipes, so a dead node sends no SIGPIPE and a full pipe can't block the job; the node
//! reads the files instead.
//!
//! - `read_log_tail`: the last lines of a log, which a diagnostic about the job's end points at (exit causes).
//! - `read_new_log_lines` and `log_line_event`: the complete lines written since an offset, as `job.log_line`
//!   events (ephemeral, contracts 7.3). The loop that follows a running job's logs every `TAIL_POLL_MS` and hands
//!   the events to `WatchLive` joins with the live queues (`live.rs`, M1-44).
//!
//! The files are the job's output, so they are read as untrusted bytes: invalid UTF-8 is replaced, not refused, and
//! a line longer than `MAX_LOG_LINE_BYTES` is cut into pieces of that size, so one huge line can't hold the reader.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rastai_core::time::now_unix_ms;
use rastai_proto::events::v1::{JobLogLine, LogStream};
use rastai_proto::node::v1::LiveEvent;
use rastai_proto::node::v1::live_event::Payload;

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

/// The last `max_lines` lines of the log at `path`, oldest first. A missing or unreadable log has none.
pub fn read_log_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let start = length.saturating_sub(TAIL_BYTES);
    let mut bytes = Vec::new();
    let read = file
        .seek(SeekFrom::Start(start))
        .and_then(|_| file.by_ref().take(TAIL_BYTES).read_to_end(&mut bytes));
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // Reading from the middle of the file starts inside a line, which would show as a cut-off first line.
    if start > 0 && !lines.is_empty() {
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

    struct TestFile {
        path: std::path::PathBuf,
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
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
        assert_eq!(read_log_tail(&file.path, 2), vec!["three", "four"]);
        assert_eq!(read_log_tail(&file.path, 10).len(), 4);
        assert!(read_log_tail(&file.path.with_extension("missing"), 3).is_empty());
    }

    // The tail of a long log starts after the cut-off line, never with half of one.
    #[test]
    fn a_long_log_tail_starts_at_a_whole_line() {
        let mut bytes = Vec::new();
        for number in 0..2000 {
            bytes.extend_from_slice(format!("line number {number}\n").as_bytes());
        }
        let file = test_file("long", &bytes);
        let tail = read_log_tail(&file.path, 10_000);
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
