//! Free disk space: `free_bytes` and `check_free_space` (layout rust-control-plane.md 6.4, flows/platform.md 8.1).
//! One free-space path for the daemon and the node alike (plan §10 row 15): nobody runs `df`.
//!
//! The only `unsafe` in the workspace (ADR-0033): std has no free-space API, so `free_bytes` makes one
//! `libc::statvfs` call, and that call is the whole `unsafe` block. `tools/checks/check-unsafe` fails on `unsafe` in
//! any other file, and `check-imports` keeps `libc` to this file.

use std::path::Path;

use crate::error::StoreError;

#[cfg(not(unix))]
const NO_FREE_SPACE_CALL_REASON: &str = "this platform has no free-space call yet";

/// Free bytes on the volume that holds `path`, for an unprivileged user: `f_bavail × f_frsize`, the space this user
/// can actually write (root's reserved blocks don't count). A failed call is `FreeSpaceUnknown`, never a guess.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn free_bytes(path: &Path) -> Result<u64, StoreError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(c_path) => c_path,
        Err(_) => return Err(free_space_unknown(path, "the path holds a NUL byte")),
    };
    // SAFETY: c_path is NUL-terminated and outlives the call; statvfs only writes into `stats`, a plain C struct
    // for which all zero bytes is a valid value.
    let (result, stats) = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        let result = libc::statvfs(c_path.as_ptr(), &mut stats);
        (result, stats)
    };
    if result != 0 {
        let reason = std::io::Error::last_os_error().to_string();
        return Err(free_space_unknown(path, &reason));
    }
    // The two fields are u64 on Linux but narrower on macOS, so the casts are needed there and no-ops here.
    #[allow(clippy::unnecessary_cast)]
    let (available_blocks, block_size) = (stats.f_bavail as u64, stats.f_frsize as u64);
    Ok(available_blocks.saturating_mul(block_size))
}

/// Windows has no body yet (ADR-0033 decision 5): it reports the space as unknown, which every check treats as
/// not enough, until the tier 2 work gives it `GetDiskFreeSpaceExW`.
#[cfg(not(unix))]
pub fn free_bytes(path: &Path) -> Result<u64, StoreError> {
    Err(free_space_unknown(path, NO_FREE_SPACE_CALL_REASON))
}

/// Ok when the volume that holds `path` has room for `needed_bytes` and still keeps `margin_bytes` free
/// (`storage.free_space_margin_bytes`, which the caller reads from settings: no policy here). Otherwise
/// `RRS-S3001`, and so is a free space that couldn't be read: unknown is "not enough", never a skipped check.
pub fn check_free_space(path: &Path, needed_bytes: u64, margin_bytes: u64) -> Result<(), StoreError> {
    let available = free_bytes(path)?;
    // Saturating: a need so large that adding the margin overflows can't fit on any disk.
    if available < needed_bytes.saturating_add(margin_bytes) {
        return Err(StoreError::NotEnoughFreeSpace {
            path: path.to_path_buf(),
            needed_bytes,
            margin_bytes,
            free_bytes: available,
        });
    }
    Ok(())
}

fn free_space_unknown(path: &Path, reason: &str) -> StoreError {
    StoreError::FreeSpaceUnknown {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}
