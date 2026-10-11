//! The JSON layer under every document: the strict reader and its positions, JSON pointers, canonical JSON for
//! content hashes, the pretty writer for files on disk, and the JSON Patch subset for undo (layout 4.1).

pub mod canonical;
pub mod patch;
pub mod pointer;
pub mod positions;
pub mod pretty;
pub mod reader;
