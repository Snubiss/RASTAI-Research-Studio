//! rastai-formats: every document type, its validation and the byte formats Rastai reads and writes
//! (layout: rust-control-plane.md section 4). M1-14 landed the byte-level base: the strict JSON reader, JSON
//! pointers, canonical JSON and content hashes, the pretty writer, the JSON Patch subset, semver, import limits and
//! byte access. M1-15 adds type descriptions, the shared header, the migration chain and `read_document` with
//! validation layers 1 and 2 (and layer 4's `check_environment`). M1-24 adds `crc32`, the one CRC-32 table, for
//! event-log frames now and zip entries later. M1-17 adds the step-1 document types and layer 3 (`validate/meaning/`).
//! M2-22 adds `zip`, the zip container read in-house with DEFLATE from `miniz_oxide`.
//! M1-22 adds `commands`, the argument and result types of the first core commands. M1-18 adds the ModelSpec
//! (`rastai.model/1`) with the TextCNN ops and their shape inference. M1-34 adds `safetensors`, the one header check
//! Rust and Python share.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod byte_source;
pub mod commands;
pub mod crc32;
pub mod describe;
pub mod documents;
pub mod error;
pub mod hashing;
pub mod header;
pub mod json;
pub mod limits;
pub mod migrate;
pub mod safetensors;
pub mod semver;
pub mod validate;
pub mod zip;

// Shared by the seeded tests in json/reader.rs and json/patch.rs.
#[cfg(test)]
mod test_random;
