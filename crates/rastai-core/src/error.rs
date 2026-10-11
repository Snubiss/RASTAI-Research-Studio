//! `CoreError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (a command
//! handler, the daemon's startup, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists. None of them has a code of its own yet, so each uses the
//! nearest meaning whose title is still true for it, as `FormatsError` does: `RandomUnavailable` ("source missing or
//! not readable": the source is `/dev/urandom`), `HomeNotFound` ("data directory not writable": with no home folder
//! there is no data directory to write) and `InvalidTimestamp` ("value is not one of the allowed values").

const CODE_SOURCE_NOT_READABLE: &str = "RRS-D3001";
const CODE_DATA_DIRECTORY_NOT_WRITABLE: &str = "RRS-S0004";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The operating system's random source couldn't be read. `reason` keeps the operating system's own words.
    RandomUnavailable { reason: String },
    /// Neither `HOME` nor the user database names a home folder, so the default data folders can't be found.
    /// Setting `RASTAI_HOME` is the way out.
    HomeNotFound,
    /// Text that isn't an RFC 3339 timestamp `time.rs` can read. `reason` says which part is wrong.
    InvalidTimestamp { text: String, reason: &'static str },
}

impl CoreError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::RandomUnavailable { .. } => CODE_SOURCE_NOT_READABLE,
            CoreError::HomeNotFound => CODE_DATA_DIRECTORY_NOT_WRITABLE,
            CoreError::InvalidTimestamp { .. } => CODE_VALUE_NOT_ALLOWED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_catalog_code() {
        let random = CoreError::RandomUnavailable {
            reason: "r".to_string(),
        };
        let timestamp = CoreError::InvalidTimestamp {
            text: "x".to_string(),
            reason: "r",
        };
        assert_eq!(random.code(), "RRS-D3001");
        assert_eq!(CoreError::HomeNotFound.code(), "RRS-S0004");
        assert_eq!(timestamp.code(), "RRS-E1004");
    }
}
