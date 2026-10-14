//! Every BLAKE3 hash in Rastai goes through this file, the only `blake3` import in the workspace (contracts 2.5;
//! deny.toml lets no other crate depend on it). Hashes are written `blake3:<64 lowercase hex>` (contracts 8.6).
//!
//! - `hash_bytes` and the stream hasher: blob names and file hashes.
//! - `content_hash_of_value`: a document's `content_hash`, BLAKE3 of its canonical JSON without that key (5.1).
//! - `model_hash`: a model's identity from its tensors, read through the safetensors reader (8.6).

// A refusal is a `Diagnostic`, as the reader returns it (layout 4.5). A refused model is rare, so it is not boxed.
#![allow(clippy::result_large_err)]

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use serde_json::Value;

use crate::byte_source::ByteSource;
use crate::error::CODE_UNPLANNABLE_LAYOUT;
use crate::json::canonical::canonical_json;
use crate::safetensors::dtypes::{SafetensorsDtype, dtype_name};
use crate::safetensors::reader::{read_safetensors_header, read_tensor_bytes};

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

/// A model's hash (contracts 8.6): BLAKE3 over one line per tensor, sorted by name, so the weights' identity doesn't
/// depend on how they are split into files, in what order the files are given, or on the metadata. Each line is
/// `<name> <dtype> <shape> <data hash>` followed by `\n`. Two files holding one tensor name are refused (`RRS-E1007`).
pub fn model_hash(weight_files: &[&dyn ByteSource]) -> Result<String, Diagnostic> {
    let mut digests = Vec::new();
    for file in weight_files {
        digests.extend(file_tensor_digests(*file)?);
    }
    digests.sort_by(|left, right| left.name.cmp(&right.name));
    refuse_repeated_names(&digests)?;
    let listing: String = digests.iter().map(tensor_line).collect();
    Ok(hash_bytes(listing.as_bytes()))
}

/// One tensor as a model hash line names it: its name, dtype, shape and the BLAKE3 of its data bytes.
struct TensorDigest {
    name: String,
    dtype: SafetensorsDtype,
    shape: Vec<u64>,
    data_hash: String,
}

/// Reads a file's header through the reader's checks, then each tensor's bytes one at a time.
fn file_tensor_digests(source: &dyn ByteSource) -> Result<Vec<TensorDigest>, Diagnostic> {
    let header = read_safetensors_header(source)?;
    let mut buffer = Vec::new();
    let mut digests = Vec::with_capacity(header.entries.len());
    for entry in &header.entries {
        read_tensor_bytes(source, &header, entry, &mut buffer)?;
        digests.push(TensorDigest {
            name: entry.name.clone(),
            dtype: entry.dtype,
            shape: entry.shape.clone(),
            data_hash: hash_bytes(&buffer),
        });
    }
    Ok(digests)
}

/// Two tensors with one name would give the listing two lines for one tensor, so the model would have no single
/// identity. Checked on the sorted list, where equal names sit next to each other.
fn refuse_repeated_names(sorted: &[TensorDigest]) -> Result<(), Diagnostic> {
    for pair in sorted.windows(2) {
        if pair[0].name == pair[1].name {
            let reason = format!("two files hold a tensor named {}", pair[0].name);
            return Err(diagnostic_at(
                CODE_UNPLANNABLE_LAYOUT,
                &[("reason", reason.as_str())],
                "",
            ));
        }
    }
    Ok(())
}

/// One listing line, ending in a newline (contracts 8.6). The name is a JSON string, so a space or a newline in a
/// name can't blur into the next field.
fn tensor_line(digest: &TensorDigest) -> String {
    format!(
        "{} {} {} {}\n",
        Value::String(digest.name.clone()),
        dtype_name(digest.dtype),
        shape_text(&digest.shape),
        digest.data_hash
    )
}

/// The dimensions as compact JSON: `[2,3]`, and `[]` for a scalar.
fn shape_text(shape: &[u64]) -> String {
    let sizes: Vec<String> = shape.iter().map(u64::to_string).collect();
    format!("[{}]", sizes.join(","))
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
    fn model_hash_of_no_files_is_the_hash_of_the_empty_listing() {
        assert_eq!(model_hash(&[]).unwrap(), EMPTY_HASH);
    }

    #[test]
    fn shape_text_is_compact_json() {
        assert_eq!(shape_text(&[2, 3]), "[2,3]");
        assert_eq!(shape_text(&[]), "[]");
    }
}
