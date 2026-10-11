//! Every BLAKE3 hash in Rastai goes through this file, the only `blake3` import in the workspace (contracts 2.5;
//! deny.toml lets no other crate depend on it). Hashes are written `blake3:<64 lowercase hex>` (contracts 8.6).
//!
//! - `hash_bytes` and the stream hasher: blob names and file hashes.
//! - `content_hash_of_value`: a document's `content_hash`, BLAKE3 of its canonical JSON without that key (5.1).
//! - `model_hash`: a model's identity from its weight files (8.6).

use serde_json::Value;

use crate::json::canonical::canonical_json;

const HASH_PREFIX: &str = "blake3:";
/// The shared-header field that holds the hash, so it is left out of what it hashes (contracts 5.1).
const CONTENT_HASH_KEY: &str = "content_hash";

/// A hash computed over bytes that arrive in pieces (a blob being written, a file read in chunks).
pub struct StreamHasher {
    hasher: blake3::Hasher,
}

/// The hash of some bytes, as `blake3:<64 hex>`.
pub fn hash_bytes(bytes: &[u8]) -> String {
    format_hash(&blake3::hash(bytes))
}

pub fn new_stream_hasher() -> StreamHasher {
    StreamHasher {
        hasher: blake3::Hasher::new(),
    }
}

pub fn update_stream_hasher(hasher: &mut StreamHasher, bytes: &[u8]) {
    hasher.hasher.update(bytes);
}

/// Equal to `hash_bytes` of everything given to `update_stream_hasher`, in order.
pub fn finish_stream_hasher(hasher: StreamHasher) -> String {
    format_hash(&hasher.hasher.finalize())
}

/// A document's content hash: canonical JSON with the top-level `content_hash` key removed, then BLAKE3. So
/// writing the hash into the file doesn't change it, and neither does spacing or key order.
pub fn content_hash_of_value(document: &Value) -> String {
    let mut without_hash = document.clone();
    if let Value::Object(members) = &mut without_hash {
        members.remove(CONTENT_HASH_KEY);
    }
    hash_bytes(canonical_json(&without_hash).as_bytes())
}

/// A model's hash from its weight files, given as (file name, file hash) pairs in any order: BLAKE3 over one
/// `<file name> <file hash>` line per file, sorted, each line ending in `\n` (contracts 8.6). Renaming,
/// adding or changing any file changes it; the order files were listed in doesn't.
pub fn model_hash(weight_files: &[(String, String)]) -> String {
    let mut lines: Vec<String> = weight_files
        .iter()
        .map(|(file_name, file_hash)| format!("{file_name} {file_hash}\n"))
        .collect();
    lines.sort();
    hash_bytes(lines.concat().as_bytes())
}

fn format_hash(hash: &blake3::Hash) -> String {
    format!("{HASH_PREFIX}{}", hash.to_hex())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    // The reference vectors from the BLAKE3 paper and the official test suite.
    const EMPTY_HASH: &str = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const ABC_HASH: &str = "blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";

    #[test]
    fn known_vectors() {
        assert_eq!(hash_bytes(b""), EMPTY_HASH);
        assert_eq!(hash_bytes(b"abc"), ABC_HASH);
    }

    #[test]
    fn stream_hash_equals_one_shot_hash() {
        let bytes: Vec<u8> = (0..100_000u32).map(|index| (index % 251) as u8).collect();
        let mut hasher = new_stream_hasher();
        for chunk in bytes.chunks(4093) {
            update_stream_hasher(&mut hasher, chunk);
        }
        assert_eq!(finish_stream_hasher(hasher), hash_bytes(&bytes));
        assert_eq!(finish_stream_hasher(new_stream_hasher()), EMPTY_HASH);
    }

    #[test]
    fn content_hash_ignores_its_own_field_spacing_and_key_order() {
        let document = json!({"type": "rastai.eval", "schema_version": 1, "tests": [1.0, 2]});
        let expected = hash_bytes(br#"{"schema_version":1,"tests":[1,2],"type":"rastai.eval"}"#);
        assert_eq!(content_hash_of_value(&document), expected);
        let with_hash =
            json!({"content_hash": "blake3:old", "tests": [1, 2.0], "type": "rastai.eval", "schema_version": 1});
        assert_eq!(content_hash_of_value(&with_hash), expected);
        // Only the top-level field is the header's; a nested `content_hash` is content.
        let nested = json!({"inner": {"content_hash": "x"}});
        assert_ne!(
            content_hash_of_value(&nested),
            content_hash_of_value(&json!({"inner": {}}))
        );
    }

    #[test]
    fn model_hash_is_sorted_lines() {
        let files = [
            ("model-00002.safetensors".to_string(), "blake3:bb".to_string()),
            ("model-00001.safetensors".to_string(), "blake3:aa".to_string()),
        ];
        let expected = hash_bytes(b"model-00001.safetensors blake3:aa\nmodel-00002.safetensors blake3:bb\n");
        assert_eq!(model_hash(&files), expected);
        let reversed = [files[1].clone(), files[0].clone()];
        assert_eq!(model_hash(&reversed), expected);
        assert_ne!(model_hash(&files[..1]), expected);
    }
}
