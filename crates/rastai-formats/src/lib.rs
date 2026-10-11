//! rastai-formats: every document type, its validation and the byte formats Rastai reads and writes
//! (layout: rust-control-plane.md section 4). M1-14 lands the byte-level base: the strict JSON reader, JSON
//! pointers, canonical JSON and content hashes, the pretty writer, the JSON Patch subset, semver, import limits and
//! byte access.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod byte_source;
pub mod error;
pub mod hashing;
pub mod json;
pub mod limits;
pub mod semver;

// Shared by the seeded tests in json/reader.rs and json/patch.rs.
#[cfg(test)]
mod test_random;
