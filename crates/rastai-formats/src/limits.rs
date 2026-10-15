//! `ImportLimits`: the caps every reader of untrusted input checks before it allocates (ADR-0032 decision 8,
//! codes `RRS-E5xxx`). The defaults are the constants below; a caller that knows better (a test, the CLI with a
//! flag) passes its own struct.
//!
//! Most numbers are first estimates; the JSON value cap's memory cost was measured. Each says why it is big enough
//! for real files and small enough to stop a hostile one; change them here, with the reason, when a real file hits
//! one.

/// Documents are hand-editable JSON: a large ModelSpec is a few MB. 64 MiB leaves room for big generated graphs.
/// Bytes alone don't bound the parsed tree's memory, because a value costs about the same whatever its length in
/// the text; DEFAULT_MAX_JSON_VALUES below bounds that.
const DEFAULT_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Real documents nest fewer than 20 levels. 128 matches serde_json's own recursion limit, and keeps the
/// recursive reader far from the stack's end on every platform (the WASM stack is 1 MiB).
const DEFAULT_MAX_JSON_DEPTH: u32 = 128;

/// Values (objects, arrays, strings, numbers, literals) in one JSON document. A real graph is a few thousand nodes
/// at a few dozen values each, well under 100,000 values; 1 million fits a 50,000-node graph at 20 values a node.
/// A graph near the 100,000-node cap below must stay under 10 values a node, so raise both together if real files
/// get there. Every value costs memory whatever its length in the text, and a non-empty object costs most: serde_json
/// gives each one a BTreeMap node of about 600 bytes.
/// Measured on x86-64, release build, peak RSS while reading (tests/reader_memory_objects.rs guards the bound): the
/// costliest shape found is 120-deep chains of one-member objects, `{"k…":{"k…":…{}…}}`. With 55-character keys
/// (60 MB of text, near both caps) it takes 801 MiB, serde_json alone 665 MiB; with 1-character keys (6 MB), 741 MiB.
/// The cap was 2 million until those chains were measured at 1.45 GiB (1.24 GiB in serde_json alone), too close to
/// the 32-bit WASM address space. A 64 MiB `[0,0,...]` stops at the cap with a 67 MiB peak (serde_json: 1 GiB).
const DEFAULT_MAX_JSON_VALUES: u64 = 1_000_000;

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
/// The reader applies it only to entries over its size floor, `RATIO_CHECK_MIN_BYTES` in `zip/reader.rs`.
const DEFAULT_MAX_ZIP_COMPRESSION_RATIO: u64 = 200;

/// The caps on untrusted input. Plain data: readers take `&ImportLimits` and compare before allocating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    /// Largest file or zip entry read whole into memory (`RRS-E5003`).
    pub max_file_bytes: u64,
    /// Deepest nesting of JSON arrays and objects; the outermost one is depth 1 (`RRS-E5004`).
    pub max_json_depth: u32,
    /// Most values in one JSON document, counting every object, array, string, number and literal (`RRS-E5006`).
    pub max_json_values: u64,
    /// Most nodes in a model graph (`RRS-E5005`).
    pub max_graph_nodes: u64,
    /// Most parameters a model document may declare (`RRS-E5005`).
    pub max_parameter_count: u64,
    /// Most entries in one zip's central directory (`RRS-E5002`).
    pub max_zip_entries: u64,
    /// Largest total uncompressed size of one zip (`RRS-E5002`).
    pub max_zip_total_bytes: u64,
    /// Largest uncompressed ÷ compressed ratio of one entry over the reader's 1 MiB floor (`RRS-E5002`).
    pub max_zip_compression_ratio: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_json_depth: DEFAULT_MAX_JSON_DEPTH,
            max_json_values: DEFAULT_MAX_JSON_VALUES,
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
        assert_eq!(limits.max_json_values, 1_000_000);
        // The ZIP64 round-trip test writes 70,000 entries (layout 4.11); the default must let it through.
        assert!(limits.max_zip_entries > 70_000);
    }
}
