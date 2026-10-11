//! Random bytes from the operating system, for ids now and secrets later (layout rust-control-plane.md 5.1).
//!
//! In-house instead of a crate: on Linux and macOS the kernel's `/dev/urandom` is the source every random crate
//! ends up reading, and reading it is a few lines. Windows (later, spec §28) needs its own system call.

use crate::error::CoreError;

#[cfg(unix)]
const RANDOM_SOURCE_PATH: &str = "/dev/urandom";

#[cfg(not(unix))]
const NO_RANDOM_SOURCE_REASON: &str = "this platform has no random source yet";

/// `count` bytes from the operating system's random source.
#[cfg(unix)]
pub fn random_bytes(count: usize) -> Result<Vec<u8>, CoreError> {
    use std::fs::File;
    use std::io::Read;

    let mut source = File::open(RANDOM_SOURCE_PATH).map_err(random_unavailable)?;
    let mut bytes = vec![0u8; count];
    source.read_exact(&mut bytes).map_err(random_unavailable)?;
    Ok(bytes)
}

/// `count` bytes from the operating system's random source. Windows support comes with the Windows build.
#[cfg(not(unix))]
pub fn random_bytes(_count: usize) -> Result<Vec<u8>, CoreError> {
    Err(CoreError::RandomUnavailable {
        reason: NO_RANDOM_SOURCE_REASON.to_string(),
    })
}

#[cfg(unix)]
fn random_unavailable(error: std::io::Error) -> CoreError {
    CoreError::RandomUnavailable {
        reason: format!("{RANDOM_SOURCE_PATH}: {error}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn gives_the_count_asked_for() {
        assert_eq!(random_bytes(0).unwrap().len(), 0);
        assert_eq!(random_bytes(10).unwrap().len(), 10);
        assert_eq!(random_bytes(4096).unwrap().len(), 4096);
    }

    // Two 32-byte draws are equal with probability 2^-256; equal draws mean the source is broken.
    #[test]
    fn two_draws_differ() {
        assert_ne!(random_bytes(32).unwrap(), random_bytes(32).unwrap());
    }
}
