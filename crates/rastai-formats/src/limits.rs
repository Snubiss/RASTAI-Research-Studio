//! `ImportLimits`: the caps every reader of untrusted input checks before it allocates (ADR-0032 decision 8,
//! codes `RRS-E5xxx`). The defaults are the constants below; a caller that knows better (a test, the CLI with a
//! flag) passes its own struct.
//!
//! The numbers are first estimates, not measurements. Each says why it is big enough for real files and small
//! enough to stop a hostile one; change them here, with the reason, when a real file hits one.

/// Documents are hand-editable JSON: a large ModelSpec is a few MB. 64 MiB leaves room for big generated graphs and
/// keeps the parsed tree (about ten times the text for serde_json values plus positions) well under a gigabyte.
const DEFAULT_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Real documents nest fewer than 20 levels. 128 matches serde_json's own recursion limit, and keeps the
/// recursive reader far from the stack's end on every platform (the WASM stack is 1 MiB).
const DEFAULT_MAX_JSON_DEPTH: u32 = 128;

/// A ModelSpec graph after block expansion: a 100-layer transformer is a few thousand nodes.
const DEFAULT_MAX_GRAPH_NODES: u64 = 100_000;

/// Parameters a ModelSpec may declare: 10 trillion is far past any model one workstation or node can hold.
const DEFAULT_MAX_PARAMETER_COUNT: u64 = 10_000_000_000_000;

/// Zip entries: the ZIP64 test archive has 70,000 (layout 4.11), and an offline project bundle holds one entry per
/// file of its weights and documents.
const DEFAULT_MAX_ZIP_ENTRIES: u64 = 200_000;

/// Total uncompressed size of a zip: offline bundles carry model weights of several GB (flows/platform.md 10.2).
const DEFAULT_MAX_ZIP_TOTAL_BYTES: u64 = 512 * 1024 * 1024 * 1024;

/// Uncompressed ÷ compressed size of one deflate entry. JSON compresses about 10:1; far beyond that is a zip bomb.
const DEFAULT_MAX_ZIP_COMPRESSION_RATIO: u64 = 200;

/// The caps on untrusted input. Plain data: readers take `&ImportLimits` and compare before allocating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    /// Largest file or zip entry read whole into memory (`RRS-E5003`).
    pub max_file_bytes: u64,
    /// Deepest nesting of JSON arrays and objects; the outermost one is depth 1 (`RRS-E5004`).
    pub max_json_depth: u32,
    /// Most nodes in a model graph (`RRS-E5005`).
    pub max_graph_nodes: u64,
    /// Most parameters a model document may declare (`RRS-E5005`).
    pub max_parameter_count: u64,
    /// Most entries in one zip's central directory (`RRS-E5002`).
    pub max_zip_entries: u64,
    /// Largest total uncompressed size of one zip (`RRS-E5002`).
    pub max_zip_total_bytes: u64,
    /// Largest uncompressed ÷ compressed ratio of one entry (`RRS-E5002`).
    pub max_zip_compression_ratio: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_json_depth: DEFAULT_MAX_JSON_DEPTH,
            max_graph_nodes: DEFAULT_MAX_GRAPH_NODES,
            max_parameter_count: DEFAULT_MAX_PARAMETER_COUNT,
            max_zip_entries: DEFAULT_MAX_ZIP_ENTRIES,
            max_zip_total_bytes: DEFAULT_MAX_ZIP_TOTAL_BYTES,
            max_zip_compression_ratio: DEFAULT_MAX_ZIP_COMPRESSION_RATIO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_constants() {
        let limits = ImportLimits::default();
        assert_eq!(limits.max_file_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_json_depth, 128);
        // The ZIP64 round-trip test writes 70,000 entries (layout 4.11); the default must let it through.
        assert!(limits.max_zip_entries > 70_000);
    }
}
