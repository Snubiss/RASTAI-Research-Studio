//! The blob store (contracts 8.4, flows/platform.md 8.2): content-addressed files under `<data>/blobs/`, each named
//! by the BLAKE3 hash of its bytes. Writes land in `.incoming/` and become visible only by a rename; a blob whose
//! bytes no longer match its name is moved to `.quarantine/` (`RRS-S1001`).
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod blob_store;
pub mod incoming;
pub mod quarantine;
