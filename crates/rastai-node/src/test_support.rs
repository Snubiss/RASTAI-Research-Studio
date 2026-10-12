//! Helpers for this crate's unit tests only (`#[cfg(test)]` in lib.rs). The integration tests can't see them and keep
//! their own in tests/common/mod.rs.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// `write_stand_in_tool` writes a script here first, beside its final path.
const STAGING_SUFFIX: &str = "staging";
const STAND_IN_TOOL_MODE: u32 = 0o755;

/// Writes an executable stand-in tool (tests/common/mod.rs has the same for the integration tests). Tests run on
/// parallel threads, and a child another thread forks inherits every descriptor this process has open until its own
/// exec; had this process written the tool, that child could still hold it open for writing when the tool runs, and
/// the kernel refuses to run it (ETXTBSY, issue #209). Renaming a finished file into place doesn't help, since the
/// child holds the same file. So `cp`, a process of its own, writes the tool from a staging file.
#[allow(clippy::unwrap_used)]
pub(crate) fn write_stand_in_tool(path: &Path, script: &str) {
    let staging = path.with_extension(STAGING_SUFFIX);
    fs::write(&staging, script).unwrap();
    let copied = Command::new("cp").arg(&staging).arg(path).status().unwrap();
    assert!(
        copied.success(),
        "cp {} {}: {copied}",
        staging.display(),
        path.display()
    );
    fs::remove_file(&staging).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(STAND_IN_TOOL_MODE)).unwrap();
}
