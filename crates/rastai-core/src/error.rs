//! `CoreError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (a command
//! handler, the daemon's startup, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists. The process-environment errors have their own codes in
//! `RRS-C` 0500–0599; a bad timestamp is a bad value in a document, so it shares the formats code for one
//! ("value is not one of the allowed values").

const CODE_NO_HOME_FOLDER: &str = "RRS-C0501";
const CODE_RANDOM_SOURCE_UNREADABLE: &str = "RRS-C0502";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The operating system's random source couldn't be read. `reason` keeps the operating system's own words.
    RandomUnavailable { reason: String },
    /// Neither `HOME`, the user database nor `RASTAI_HOME` names a folder, so there is nowhere to put the data
    /// folders. Setting `RASTAI_HOME` is the way out (`read_path_environment` uses it in place of the home folder).
    HomeNotFound,
    /// Text that isn't an RFC 3339 timestamp `time.rs` can read. `reason` says which part is wrong.
    InvalidTimestamp { text: String, reason: &'static str },
}

impl CoreError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::RandomUnavailable { .. } => CODE_RANDOM_SOURCE_UNREADABLE,
            CoreError::HomeNotFound => CODE_NO_HOME_FOLDER,
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
        assert_eq!(random.code(), "RRS-C0502");
        assert_eq!(CoreError::HomeNotFound.code(), "RRS-C0501");
        assert_eq!(timestamp.code(), "RRS-E1004");
    }
}
