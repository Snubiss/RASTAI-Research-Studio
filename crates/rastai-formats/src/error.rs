//! `FormatsError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge
//! (layer 1 in validate/syntax.rs, a command handler, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 already lists. Five of them have no code of their own yet and
//! borrow the nearest meaning (`InvalidVersion`, `PatchFailed`, `NotSerializable`, `ReadFailed`, `WriteFailed`);
//! a dedicated code is a catalog contract change, made in its own PR.

use crate::json::positions::TextPosition;

const CODE_JSON_SYNTAX: &str = "RRS-E0001";
const CODE_NOT_UTF8: &str = "RRS-E0002";
const CODE_DUPLICATE_KEY: &str = "RRS-E0003";
const CODE_NOT_A_NUMBER: &str = "RRS-E0004";
const CODE_WRONG_KIND: &str = "RRS-E1002";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";
const CODE_REFERENCE_UNRESOLVED: &str = "RRS-E2001";
const CODE_FILE_TOO_LARGE: &str = "RRS-E5003";
const CODE_TOO_DEEP: &str = "RRS-E5004";
const CODE_SOURCE_NOT_READABLE: &str = "RRS-D3001";
const CODE_DISK_FULL_DURING_WRITE: &str = "RRS-S3002";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatsError {
    /// The text isn't JSON at this position. `expected` says what the reader wanted there ("a value", "',' or
    /// '}'", "end of input"), for the message placeholder and for logs.
    JsonSyntax {
        position: TextPosition,
        expected: &'static str,
    },
    /// The bytes aren't UTF-8. The position is the first bad byte's character position.
    NotUtf8 { position: TextPosition },
    /// An object has the same key twice. `pointer` names the second one.
    DuplicateKey { pointer: String, position: TextPosition },
    /// `NaN`, `Infinity`, `-Infinity`, or a number too large for a 64-bit float (it would read as Infinity).
    NotANumber { position: TextPosition },
    /// The input is bigger than `ImportLimits.max_file_bytes`; checked before anything is read or allocated.
    FileTooLarge { size_bytes: u64, limit_bytes: u64 },
    /// Arrays and objects nest deeper than `ImportLimits.max_json_depth`. The position is the bracket that went
    /// past the limit.
    TooDeep { position: TextPosition, limit_depth: u32 },
    /// A version or version range that semver.rs can't read. Borrows "value is not one of the allowed values".
    InvalidVersion { text: String, reason: &'static str },
    /// A JSON Patch operation whose path or `from` doesn't resolve in the document. Borrows "reference doesn't
    /// resolve": the pointer is the reference.
    PatchFailed {
        operation_index: usize,
        path: String,
        reason: &'static str,
    },
    /// serde refused to write a value (a map with non-string keys). Borrows "field has the wrong kind".
    NotSerializable { reason: String },
    /// A `ByteSource` couldn't give the bytes asked for: past the end, or an I/O error. Borrows "source missing or
    /// not readable".
    ReadFailed { offset: u64, length: u64, reason: String },
    /// A `ByteSink` couldn't write. Borrows "disk full during a write", the usual cause; `reason` keeps the
    /// operating system's own words.
    WriteFailed { reason: String },
}

impl FormatsError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            FormatsError::JsonSyntax { .. } => CODE_JSON_SYNTAX,
            FormatsError::NotUtf8 { .. } => CODE_NOT_UTF8,
            FormatsError::DuplicateKey { .. } => CODE_DUPLICATE_KEY,
            FormatsError::NotANumber { .. } => CODE_NOT_A_NUMBER,
            FormatsError::FileTooLarge { .. } => CODE_FILE_TOO_LARGE,
            FormatsError::TooDeep { .. } => CODE_TOO_DEEP,
            FormatsError::InvalidVersion { .. } => CODE_VALUE_NOT_ALLOWED,
            FormatsError::PatchFailed { .. } => CODE_REFERENCE_UNRESOLVED,
            FormatsError::NotSerializable { .. } => CODE_WRONG_KIND,
            FormatsError::ReadFailed { .. } => CODE_SOURCE_NOT_READABLE,
            FormatsError::WriteFailed { .. } => CODE_DISK_FULL_DURING_WRITE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: TextPosition = TextPosition { line: 1, column: 1 };

    fn assert_codes(cases: Vec<(FormatsError, &str)>) {
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error:?}");
        }
    }

    #[test]
    fn reader_errors_have_their_catalog_codes() {
        let syntax = FormatsError::JsonSyntax {
            position: START,
            expected: "a value",
        };
        let duplicate = FormatsError::DuplicateKey {
            pointer: "/a".to_string(),
            position: START,
        };
        let too_large = FormatsError::FileTooLarge {
            size_bytes: 2,
            limit_bytes: 1,
        };
        let too_deep = FormatsError::TooDeep {
            position: START,
            limit_depth: 1,
        };
        assert_codes(vec![
            (syntax, "RRS-E0001"),
            (FormatsError::NotUtf8 { position: START }, "RRS-E0002"),
            (duplicate, "RRS-E0003"),
            (FormatsError::NotANumber { position: START }, "RRS-E0004"),
            (too_large, "RRS-E5003"),
            (too_deep, "RRS-E5004"),
        ]);
    }

    #[test]
    fn borrowed_codes_are_the_nearest_existing_meaning() {
        let version = FormatsError::InvalidVersion {
            text: "x".to_string(),
            reason: "r",
        };
        let patch = FormatsError::PatchFailed {
            operation_index: 0,
            path: "/a".to_string(),
            reason: "r",
        };
        let read = FormatsError::ReadFailed {
            offset: 0,
            length: 1,
            reason: "r".to_string(),
        };
        let reason = "r".to_string();
        assert_codes(vec![
            (version, "RRS-E1004"),
            (patch, "RRS-E2001"),
            (FormatsError::NotSerializable { reason: reason.clone() }, "RRS-E1002"),
            (read, "RRS-D3001"),
            (FormatsError::WriteFailed { reason }, "RRS-S3002"),
        ]);
    }
}
