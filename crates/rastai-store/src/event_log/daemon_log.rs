//! The segments of the daemon's own log (contracts 7.1, layout 13.1) and their `log_cursors` names. The live file is
//! `<data>/daemon/events.log`; a full one is renamed `events-<n>.log` and a new one started. Each segment has its own
//! sequence and its own cursor, `daemon:<n>`, named from the day it is started, so the name survives the rename: the
//! live file is segment (rolled segments + 1).
//!
//! The daemon writes and replays the log through these (rastai-server `daemon_events.rs`, `daemon_log_replay.rs`)
//! and the offline rebuild reads it through them (`rebuild.rs`), so both name every segment the same way.

use std::path::PathBuf;

use rastai_core::paths::{DataPaths, daemon_events_log_path, daemon_events_segment_path};

/// `log_cursors.log_name` of a daemon-log segment is this and its number.
const DAEMON_LOG_NAME_PREFIX: &str = "daemon:";
const FIRST_SEGMENT_NUMBER: u64 = 1;

/// One segment of the daemon log: its number (from 1), its cursor name and its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonLogSegment {
    pub segment_number: u64,
    pub log_name: String,
    pub path: PathBuf,
}

/// Every segment in log order: the rolled ones (`events-1.log`, `events-2.log`, ...), then the live `events.log`,
/// which may not exist yet.
pub fn daemon_log_segments(paths: &DataPaths) -> Vec<DaemonLogSegment> {
    let mut segments = Vec::new();
    let mut segment_number = FIRST_SEGMENT_NUMBER;
    loop {
        let path = daemon_events_segment_path(paths, segment_number);
        if !path.exists() {
            break;
        }
        segments.push(DaemonLogSegment {
            segment_number,
            log_name: daemon_log_name(segment_number),
            path,
        });
        segment_number += 1;
    }
    segments.push(DaemonLogSegment {
        segment_number,
        log_name: daemon_log_name(segment_number),
        path: daemon_events_log_path(paths),
    });
    segments
}

/// `daemon:<n>`, the `log_cursors` name of segment `n`.
pub fn daemon_log_name(segment_number: u64) -> String {
    format!("{DAEMON_LOG_NAME_PREFIX}{segment_number}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_live_file_comes_after_the_rolled_segments() {
        let root = std::env::temp_dir().join(format!("rastai-store-daemon-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        };
        let only_live = daemon_log_segments(&paths);
        assert_eq!(only_live.len(), 1);
        assert_eq!(only_live[0].log_name, "daemon:1");
        assert_eq!(only_live[0].segment_number, 1);
        assert_eq!(only_live[0].path, daemon_events_log_path(&paths));

        std::fs::create_dir_all(paths.data.join("daemon")).unwrap();
        std::fs::write(daemon_events_segment_path(&paths, 1), b"").unwrap();
        std::fs::write(daemon_events_segment_path(&paths, 2), b"").unwrap();

        let segments = daemon_log_segments(&paths);

        let names: Vec<&str> = segments.iter().map(|segment| segment.log_name.as_str()).collect();
        assert_eq!(names, ["daemon:1", "daemon:2", "daemon:3"]);
        assert_eq!(segments[2].segment_number, 3);
        assert_eq!(segments[2].path, daemon_events_log_path(&paths));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
