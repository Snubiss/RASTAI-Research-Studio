//! The one safetensors header check (ADR-0029 decision 2, ADR-0037 decision 3, contracts 2.4 rule 10). Rust calls
//! it directly; Python calls it through the wheel (`rastai_formats.check_safetensors_header`) before it reads a
//! single tensor byte, so both languages give the same verdict and the same `RRS-E60xx` code on every file.
//!
//! A safetensors file is an 8-byte little-endian header length, that many bytes of JSON header, then the data
//! section. Weight files come from the Hub, imports and other people, so all of it is untrusted. A file is refused
//! unless:
//! - the header length is at most `MAX_HEADER_BYTES` and fits inside the file, checked before anything is read or
//!   allocated (`read_safetensors_header_length`);
//! - the header is UTF-8 JSON with no duplicate keys, no `NaN` or `Infinity` and nothing after the object (the
//!   in-house reader every document uses), and the JSON is an object;
//! - `__metadata__`, if present, maps strings to strings;
//! - every other member is an object with a known dtype, a shape of real non-negative integers (a boolean or a
//!   float is not an integer) and `data_offsets` of exactly two real integers inside the data section;
//! - each tensor's byte size, product(shape) × element size computed with overflow checks, equals end − start: a
//!   shape of [2^32, 2^32] must not wrap around to 0 and "match" empty offsets;
//! - the tensors tile the data section exactly: no overlap, no gap, no bytes left over at the end.
//!
//! The first problem found is the answer, one code per kind of problem (contracts 10.3). The order is fixed (the
//! metadata, then each tensor by name, then the tiling), so a file with several problems always gets the same code.
//!
//! The reader over a `ByteSource`, which streams tensor bytes in Rust, is M2-13's (`reader.rs`); it will call these
//! two functions too.

// A refusal is a `Diagnostic`, the contract type every caller renders (the layout's signatures, section 4.5); it is
// larger than clippy's default for an `Err`, and a refused file is rare, so it is returned as it is, not boxed.
#![allow(clippy::result_large_err)]

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_diag::path::{DiagnosticPath, format_path};
use serde_json::{Map, Value};

use crate::error::FormatsError;
use crate::json::pointer::{ROOT_POINTER, join_pointer, split_pointer};
use crate::json::reader::read_json;
use crate::limits::ImportLimits;
use crate::safetensors::dtypes::{SafetensorsDtype, dtype_element_bytes, dtype_from_name};
use crate::validate::syntax::syntax_diagnostic;

/// The header length at the start of every file: a little-endian u64.
pub const HEADER_LENGTH_BYTES: u64 = 8;
const LENGTH_PREFIX_BYTES: usize = 8;
/// The reference implementation's cap (spike 07). Checked before the header is read, so a lying length can't make
/// anyone allocate 2^63 bytes.
pub const MAX_HEADER_BYTES: u64 = 100 * 1024 * 1024;
/// The one header member that isn't a tensor.
pub const METADATA_KEY: &str = "__metadata__";
const DTYPE_KEY: &str = "dtype";
const SHAPE_KEY: &str = "shape";
const OFFSETS_KEY: &str = "data_offsets";
const OFFSET_COUNT: usize = 2;
/// 2^64. The JSON reader keeps an integer that fits u64 as an integer and turns a bigger one into a float, so a
/// number at or past this is past 64 bits however it was written (`100000000000000000000000000000` or `1e29`).
const FIRST_NUMBER_PAST_64_BITS: f64 = 18_446_744_073_709_551_616.0;
/// What a missing dtype shows as in the message.
const MISSING_VALUE_TEXT: &str = "null";

// The codes of contracts 10.3, one per kind of problem.
const CODE_FILE_TOO_SHORT: &str = "RRS-E6001";
const CODE_LENGTH_PAST_END: &str = "RRS-E6002";
const CODE_HEADER_TOO_LARGE: &str = "RRS-E6003";
const CODE_NOT_UTF8_JSON: &str = "RRS-E6004";
const CODE_NOT_AN_OBJECT: &str = "RRS-E6005";
const CODE_DUPLICATE_KEY: &str = "RRS-E6006";
const CODE_ENTRY_NOT_OBJECT: &str = "RRS-E6007";
const CODE_UNSUPPORTED_DTYPE: &str = "RRS-E6008";
const CODE_BAD_SHAPE: &str = "RRS-E6009";
const CODE_BAD_OFFSETS: &str = "RRS-E6010";
const CODE_OFFSETS_OUTSIDE: &str = "RRS-E6011";
const CODE_SIZE_OVERFLOWS: &str = "RRS-E6012";
const CODE_SIZE_MISMATCH: &str = "RRS-E6013";
const CODE_OVERLAP: &str = "RRS-E6014";
const CODE_GAP: &str = "RRS-E6015";
const CODE_TRAILING_BYTES: &str = "RRS-E6016";
const CODE_BAD_METADATA: &str = "RRS-E6017";

/// One tensor of an accepted header. `start` and `end` are byte offsets into the data section, not the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorEntry {
    pub name: String,
    pub dtype: SafetensorsDtype,
    pub shape: Vec<u64>,
    pub start: u64,
    pub end: u64,
}

/// An accepted header. `entries` are in data-section order; `data_start` is where the data section starts in the
/// file (the 8-byte length plus the header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetensorsHeader {
    pub entries: Vec<TensorEntry>,
    pub metadata: Vec<(String, String)>,
    pub data_start: u64,
}

/// A JSON number read as a size or an offset.
enum HeaderCount {
    Fits(u64),
    Past64Bits,
    NotACount,
}

/// The header length from the first 8 bytes of a file `file_length` bytes long. The caller reads the header bytes
/// only after this accepts them: the file is at least 8 bytes long (`RRS-E6001`), the header is at most
/// `MAX_HEADER_BYTES` (`RRS-E6003`) and it ends inside the file (`RRS-E6002`).
pub fn read_safetensors_header_length(length_prefix: &[u8], file_length: u64) -> Result<u64, Diagnostic> {
    let Some(prefix) = length_prefix.first_chunk::<LENGTH_PREFIX_BYTES>() else {
        return Err(header_diagnostic(CODE_FILE_TOO_SHORT, &[], ""));
    };
    if file_length < HEADER_LENGTH_BYTES {
        return Err(header_diagnostic(CODE_FILE_TOO_SHORT, &[], ""));
    }
    let header_length = u64::from_le_bytes(*prefix);
    let header_bytes_text = header_length.to_string();
    let values = [("header_bytes", header_bytes_text.as_str())];
    if header_length > MAX_HEADER_BYTES {
        return Err(header_diagnostic(CODE_HEADER_TOO_LARGE, &values, ""));
    }
    // No overflow: header_length is at most MAX_HEADER_BYTES here.
    if HEADER_LENGTH_BYTES + header_length > file_length {
        return Err(header_diagnostic(CODE_LENGTH_PAST_END, &values, ""));
    }
    Ok(header_length)
}

/// Checks a header against the strict contract. `header_bytes` is the JSON after the 8-byte length and
/// `data_length` the size of the data section after it. Every rule is checked before anything is accepted, so a
/// caller may read tensor bytes only after this returns Ok.
pub fn check_safetensors_header(header_bytes: &[u8], data_length: u64) -> Result<SafetensorsHeader, Diagnostic> {
    let header_length = header_bytes.len() as u64;
    if header_length > MAX_HEADER_BYTES {
        let header_bytes_text = header_length.to_string();
        return Err(header_diagnostic(
            CODE_HEADER_TOO_LARGE,
            &[("header_bytes", &header_bytes_text)],
            "",
        ));
    }
    let members = read_header_object(header_bytes)?;
    let metadata = read_metadata(members.get(METADATA_KEY))?;
    let mut entries = Vec::new();
    for name in tensor_names(&members) {
        entries.push(read_entry(name, &members[name], data_length)?);
    }
    check_exact_tiling(&mut entries, data_length)?;
    Ok(SafetensorsHeader {
        entries,
        metadata,
        data_start: HEADER_LENGTH_BYTES + header_length,
    })
}

/// The header's members, through the strict reader every document uses, so "valid JSON" means the same here as
/// everywhere (spike 07: a header `NaN` and trailing garbage are syntax errors in every reader).
fn read_header_object(header_bytes: &[u8]) -> Result<Map<String, Value>, Diagnostic> {
    let limits = ImportLimits {
        max_file_bytes: MAX_HEADER_BYTES,
        ..ImportLimits::default()
    };
    let parsed = read_json(header_bytes, &limits).map_err(|error| reader_diagnostic(&error))?;
    match parsed.value {
        Value::Object(members) => Ok(members),
        _ => Err(header_diagnostic(CODE_NOT_AN_OBJECT, &[], "")),
    }
}

/// Every member but `__metadata__`, sorted, so the first problem reported doesn't depend on how the map stores
/// its keys.
fn tensor_names(members: &Map<String, Value>) -> Vec<&String> {
    let mut names: Vec<&String> = members.keys().filter(|name| *name != METADATA_KEY).collect();
    names.sort();
    names
}

/// A JSON reader error as a header diagnostic. The value-count and depth caps keep their own `RRS-E5xxx` codes:
/// they are import limits, not a broken file.
fn reader_diagnostic(error: &FormatsError) -> Diagnostic {
    match error {
        FormatsError::DuplicateKey { pointer, .. } => {
            let key = last_pointer_token(pointer);
            header_diagnostic(CODE_DUPLICATE_KEY, &[("key", &key)], pointer)
        }
        FormatsError::TooDeep { .. } | FormatsError::TooManyValues { .. } => syntax_diagnostic(error, ""),
        _ => header_diagnostic(CODE_NOT_UTF8_JSON, &[], ""),
    }
}

/// The key a duplicate-key pointer ends in, unescaped: `/a~1b` is `a/b`.
fn last_pointer_token(pointer: &str) -> String {
    let tokens = split_pointer(pointer).unwrap_or_default();
    tokens.last().cloned().unwrap_or_default()
}

/// `__metadata__` as (key, value) pairs; absent is the same as empty.
fn read_metadata(metadata: Option<&Value>) -> Result<Vec<(String, String)>, Diagnostic> {
    let Some(metadata) = metadata else {
        return Ok(Vec::new());
    };
    let bad_metadata = || header_diagnostic(CODE_BAD_METADATA, &[], &join_pointer(ROOT_POINTER, METADATA_KEY));
    let Value::Object(members) = metadata else {
        return Err(bad_metadata());
    };
    let mut pairs = Vec::with_capacity(members.len());
    for (key, value) in members {
        let Value::String(text) = value else {
            return Err(bad_metadata());
        };
        pairs.push((key.clone(), text.clone()));
    }
    Ok(pairs)
}

/// One tensor's member, checked in a fixed order: object, dtype, shape, offsets, offsets inside the data section,
/// byte size.
fn read_entry(name: &str, description: &Value, data_length: u64) -> Result<TensorEntry, Diagnostic> {
    let Value::Object(fields) = description else {
        return Err(tensor_diagnostic(
            CODE_ENTRY_NOT_OBJECT,
            name,
            &[],
            &tensor_pointer(name, ""),
        ));
    };
    let dtype = read_dtype(name, fields)?;
    let shape_counts = read_shape(name, fields)?;
    let (start, end) = read_offsets(name, fields, data_length)?;
    let shape = shape_sizes(name, &shape_counts)?;
    let expected_bytes = shape_byte_size(name, dtype, &shape)?;
    let found_bytes = end - start;
    if expected_bytes != found_bytes {
        let expected_text = expected_bytes.to_string();
        let found_text = found_bytes.to_string();
        let values = [
            ("expected_bytes", expected_text.as_str()),
            ("found_bytes", found_text.as_str()),
        ];
        return Err(tensor_diagnostic(
            CODE_SIZE_MISMATCH,
            name,
            &values,
            &tensor_pointer(name, ""),
        ));
    }
    Ok(TensorEntry {
        name: name.to_string(),
        dtype,
        shape,
        start,
        end,
    })
}

fn read_dtype(name: &str, fields: &Map<String, Value>) -> Result<SafetensorsDtype, Diagnostic> {
    let found = fields.get(DTYPE_KEY);
    if let Some(Value::String(dtype_text)) = found
        && let Some(dtype) = dtype_from_name(dtype_text)
    {
        return Ok(dtype);
    }
    let shown = match found {
        Some(Value::String(dtype_text)) => dtype_text.clone(),
        Some(other) => other.to_string(),
        None => MISSING_VALUE_TEXT.to_string(),
    };
    let pointer = tensor_pointer(name, DTYPE_KEY);
    Err(tensor_diagnostic(
        CODE_UNSUPPORTED_DTYPE,
        name,
        &[("dtype", &shown)],
        &pointer,
    ))
}

/// The shape as counts. A size past 64 bits is kept for now: it is a byte-size overflow (`RRS-E6012`), reported
/// after the offsets, like any other overflow. Anything that isn't a non-negative integer is `RRS-E6009` here.
fn read_shape(name: &str, fields: &Map<String, Value>) -> Result<Vec<HeaderCount>, Diagnostic> {
    let bad_shape = || tensor_diagnostic(CODE_BAD_SHAPE, name, &[], &tensor_pointer(name, SHAPE_KEY));
    let Some(Value::Array(sizes)) = fields.get(SHAPE_KEY) else {
        return Err(bad_shape());
    };
    let mut counts = Vec::with_capacity(sizes.len());
    for size in sizes {
        let count = read_count(size);
        if let HeaderCount::NotACount = count {
            return Err(bad_shape());
        }
        counts.push(count);
    }
    Ok(counts)
}

/// `[start, end]` with `start <= end <= data_length`. Two real integers or `RRS-E6010`; reversed, past the end of
/// the data section or past 64 bits is `RRS-E6011`.
fn read_offsets(name: &str, fields: &Map<String, Value>, data_length: u64) -> Result<(u64, u64), Diagnostic> {
    let pointer = tensor_pointer(name, OFFSETS_KEY);
    let bad_offsets = || tensor_diagnostic(CODE_BAD_OFFSETS, name, &[], &pointer);
    let outside = || tensor_diagnostic(CODE_OFFSETS_OUTSIDE, name, &[], &pointer);
    let Some(Value::Array(offsets)) = fields.get(OFFSETS_KEY) else {
        return Err(bad_offsets());
    };
    if offsets.len() != OFFSET_COUNT {
        return Err(bad_offsets());
    }
    let start = read_count(&offsets[0]);
    let end = read_count(&offsets[1]);
    match (start, end) {
        (HeaderCount::NotACount, _) | (_, HeaderCount::NotACount) => Err(bad_offsets()),
        (HeaderCount::Fits(start), HeaderCount::Fits(end)) if start <= end && end <= data_length => Ok((start, end)),
        _ => Err(outside()),
    }
}

/// The shape's sizes once every one fits 64 bits; a size past 64 bits makes the byte size overflow.
fn shape_sizes(name: &str, counts: &[HeaderCount]) -> Result<Vec<u64>, Diagnostic> {
    let mut sizes = Vec::with_capacity(counts.len());
    for count in counts {
        match count {
            HeaderCount::Fits(size) => sizes.push(*size),
            _ => return Err(overflow_diagnostic(name)),
        }
    }
    Ok(sizes)
}

/// Element size × every size, left to right, with checked_mul: any step past 64 bits is `RRS-E6012`, even when a
/// later size is 0. Refusing is the safe side, and no real tensor gets near it.
fn shape_byte_size(name: &str, dtype: SafetensorsDtype, shape: &[u64]) -> Result<u64, Diagnostic> {
    let mut byte_size = dtype_element_bytes(dtype);
    for size in shape {
        byte_size = byte_size.checked_mul(*size).ok_or_else(|| overflow_diagnostic(name))?;
    }
    Ok(byte_size)
}

fn overflow_diagnostic(name: &str) -> Diagnostic {
    tensor_diagnostic(CODE_SIZE_OVERFLOWS, name, &[], &tensor_pointer(name, SHAPE_KEY))
}

/// The tensors, sorted into data-section order, must cover it exactly. An overlap would let one tensor alias
/// another; a gap or leftover bytes would be data no tensor accounts for. Ties (empty tensors at one offset) sort
/// by name, so the order is the same on every run.
fn check_exact_tiling(entries: &mut [TensorEntry], data_length: u64) -> Result<(), Diagnostic> {
    entries.sort_by(|left, right| (left.start, left.end, &left.name).cmp(&(right.start, right.end, &right.name)));
    let mut expected_start = 0;
    let mut previous_name = "";
    for entry in entries.iter() {
        let pointer = tensor_pointer(&entry.name, "");
        if entry.start < expected_start {
            let values = [("other_tensor_name", previous_name)];
            return Err(tensor_diagnostic(CODE_OVERLAP, &entry.name, &values, &pointer));
        }
        if entry.start > expected_start {
            let offset_text = expected_start.to_string();
            return Err(header_diagnostic(CODE_GAP, &[("offset", &offset_text)], &pointer));
        }
        expected_start = entry.end;
        previous_name = &entry.name;
    }
    if expected_start != data_length {
        let extra_text = (data_length - expected_start).to_string();
        return Err(header_diagnostic(
            CODE_TRAILING_BYTES,
            &[("extra_bytes", &extra_text)],
            "",
        ));
    }
    Ok(())
}

/// A size or offset: an integer that fits u64, a number past 64 bits, or anything else (a negative or fractional
/// number, a boolean, a string, null, a list).
fn read_count(value: &Value) -> HeaderCount {
    if let Some(count) = value.as_u64() {
        return HeaderCount::Fits(count);
    }
    match value.as_f64() {
        Some(number) if number >= FIRST_NUMBER_PAST_64_BITS => HeaderCount::Past64Bits,
        _ => HeaderCount::NotACount,
    }
}

/// `/<tensor name>` or `/<tensor name>/<key>`, the pointer a diagnostic names.
fn tensor_pointer(name: &str, key: &str) -> String {
    let tensor = join_pointer(ROOT_POINTER, name);
    if key.is_empty() {
        return tensor;
    }
    join_pointer(&tensor, key)
}

fn tensor_diagnostic(code: &str, name: &str, other_values: &[(&str, &str)], pointer: &str) -> Diagnostic {
    let mut values = vec![("tensor_name", name)];
    values.extend_from_slice(other_values);
    header_diagnostic(code, &values, pointer)
}

/// A diagnostic whose path is a JSON pointer into the header (`#/a/shape`), or empty for the whole file. The
/// header has no file name of its own here; the caller knows which file it read.
fn header_diagnostic(code: &str, values: &[(&str, &str)], pointer: &str) -> Diagnostic {
    let path = DiagnosticPath {
        json_pointer: pointer.to_string(),
        ..DiagnosticPath::default()
    };
    diagnostic_at(code, values, &format_path(&path))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The code and path of a refused header.
    fn refusal(header: &str, data_length: u64) -> (String, String) {
        let diagnostic = check_safetensors_header(header.as_bytes(), data_length).unwrap_err();
        (diagnostic.code, diagnostic.path)
    }

    fn pair(code: &str, path: &str) -> (String, String) {
        (code.to_string(), path.to_string())
    }

    fn file_prefix(header_length: u64) -> Vec<u8> {
        header_length.to_le_bytes().to_vec()
    }

    #[test]
    fn a_valid_header_gives_its_entries_in_data_order_and_its_metadata() {
        let header = r#"{"b": {"dtype": "BF16", "shape": [4], "data_offsets": [24, 32]},
            "a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 24]},
            "__metadata__": {"format": "pt"}}"#;
        let accepted = check_safetensors_header(header.as_bytes(), 32).unwrap();
        let names: Vec<&str> = accepted.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(accepted.entries[0].dtype, SafetensorsDtype::F32);
        assert_eq!(accepted.entries[0].shape, [2, 3]);
        assert_eq!((accepted.entries[1].start, accepted.entries[1].end), (24, 32));
        assert_eq!(accepted.metadata, [("format".to_string(), "pt".to_string())]);
        assert_eq!(accepted.data_start, HEADER_LENGTH_BYTES + header.len() as u64);
    }

    #[test]
    fn empty_tensors_at_one_offset_tile_in_name_order() {
        let header = r#"{"z": {"dtype": "U8", "shape": [0], "data_offsets": [0, 0]},
            "y": {"dtype": "U8", "shape": [2, 0], "data_offsets": [0, 0]},
            "x": {"dtype": "U8", "shape": [1], "data_offsets": [0, 1]}}"#;
        let accepted = check_safetensors_header(header.as_bytes(), 1).unwrap();
        let names: Vec<&str> = accepted.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["y", "z", "x"]);
    }

    #[test]
    fn the_length_prefix_is_checked_before_anything_is_read() {
        let short = read_safetensors_header_length(&[16, 0, 0, 0], 4).unwrap_err();
        assert_eq!(short.code, "RRS-E6001");
        let over = read_safetensors_header_length(&file_prefix(MAX_HEADER_BYTES + 1), u64::MAX).unwrap_err();
        assert_eq!(over.code, "RRS-E6003");
        let past_end = read_safetensors_header_length(&file_prefix(10), 17).unwrap_err();
        assert_eq!(past_end.code, "RRS-E6002");
        assert!(past_end.message.contains("10 bytes"), "{}", past_end.message);
        assert_eq!(read_safetensors_header_length(&file_prefix(10), 18).unwrap(), 10);
        assert_eq!(read_safetensors_header_length(&file_prefix(0), 8).unwrap(), 0);
    }

    #[test]
    fn a_prefix_shorter_than_eight_bytes_is_a_short_file_whatever_length_is_claimed() {
        let refused = read_safetensors_header_length(&[0; 7], 1_000).unwrap_err();
        assert_eq!(refused.code, "RRS-E6001");
    }

    #[test]
    fn a_header_over_the_cap_is_refused_before_it_is_parsed() {
        let header = vec![b' '; (MAX_HEADER_BYTES + 1) as usize];
        let refused = check_safetensors_header(&header, 0).unwrap_err();
        assert_eq!(refused.code, "RRS-E6003");
    }

    #[test]
    fn json_problems_have_one_code_and_a_duplicate_key_names_its_place() {
        assert_eq!(refusal("{} x", 0), pair("RRS-E6004", ""));
        assert_eq!(refusal(r#"{"a": NaN}"#, 0), pair("RRS-E6004", ""));
        let not_utf8 = check_safetensors_header(b"{\"a\xff\": 1}", 0).unwrap_err();
        assert_eq!(not_utf8.code, "RRS-E6004");
        assert_eq!(refusal("[]", 0), pair("RRS-E6005", ""));
        let nested = r#"{"a": {"dtype": "U8", "dtype": "U8", "shape": [0], "data_offsets": [0, 0]}}"#;
        assert_eq!(refusal(nested, 0), pair("RRS-E6006", "#/a/dtype"));
        let duplicate = check_safetensors_header(br#"{"a~b": 1, "a~b": 2}"#, 0).unwrap_err();
        assert_eq!(duplicate.path, "#/a~0b");
        assert!(duplicate.message.contains("a~b"), "{}", duplicate.message);
    }

    #[test]
    fn the_json_caps_keep_their_import_limit_codes() {
        let place = crate::json::positions::TextPosition { line: 1, column: 2 };
        let too_many = FormatsError::TooManyValues {
            position: place,
            limit_count: 1,
        };
        assert_eq!(reader_diagnostic(&too_many).code, "RRS-E5006");
        let too_deep = FormatsError::TooDeep {
            position: place,
            limit_depth: 1,
        };
        assert_eq!(reader_diagnostic(&too_deep).code, "RRS-E5004");
    }

    #[test]
    fn entry_problems_point_at_their_field() {
        assert_eq!(refusal(r#"{"a": 5}"#, 0), pair("RRS-E6007", "#/a"));
        let no_dtype = r#"{"a": {"shape": [1], "data_offsets": [0, 1]}}"#;
        assert_eq!(refusal(no_dtype, 1), pair("RRS-E6008", "#/a/dtype"));
        let bool_size = r#"{"a": {"dtype": "U8", "shape": [true], "data_offsets": [0, 1]}}"#;
        assert_eq!(refusal(bool_size, 1), pair("RRS-E6009", "#/a/shape"));
        let negative = r#"{"a": {"dtype": "U8", "shape": [-1], "data_offsets": [0, 0]}}"#;
        assert_eq!(refusal(negative, 0), pair("RRS-E6009", "#/a/shape"));
        let bool_offset = r#"{"a": {"dtype": "U8", "shape": [1], "data_offsets": [false, 1]}}"#;
        assert_eq!(refusal(bool_offset, 1), pair("RRS-E6010", "#/a/data_offsets"));
        let three = r#"{"a": {"dtype": "U8", "shape": [1], "data_offsets": [0, 1, 1]}}"#;
        assert_eq!(refusal(three, 1), pair("RRS-E6010", "#/a/data_offsets"));
    }

    #[test]
    fn an_unsupported_dtype_is_named_in_the_message() {
        let header = r#"{"a": {"dtype": "F128", "shape": [1], "data_offsets": [0, 16]}}"#;
        let refused = check_safetensors_header(header.as_bytes(), 16).unwrap_err();
        assert!(refused.message.contains("F128"), "{}", refused.message);
    }

    #[test]
    fn numbers_past_64_bits_are_outside_offsets_and_overflowing_shapes() {
        let offset = r#"{"a": {"dtype": "U8", "shape": [1], "data_offsets": [0, 1e30]}}"#;
        assert_eq!(refusal(offset, 1), pair("RRS-E6011", "#/a/data_offsets"));
        let size = r#"{"a": {"dtype": "U8", "shape": [18446744073709551616], "data_offsets": [0, 0]}}"#;
        assert_eq!(refusal(size, 0), pair("RRS-E6012", "#/a/shape"));
        let wraps = r#"{"a": {"dtype": "U8", "shape": [4294967296, 4294967296], "data_offsets": [0, 0]}}"#;
        assert_eq!(refusal(wraps, 0), pair("RRS-E6012", "#/a/shape"));
        let wraps_before_zero =
            r#"{"a": {"dtype": "F32", "shape": [4294967296, 4294967296, 0], "data_offsets": [0, 0]}}"#;
        assert_eq!(refusal(wraps_before_zero, 0).0, "RRS-E6012");
        let small_float = r#"{"a": {"dtype": "U8", "shape": [1], "data_offsets": [0, 1.0]}}"#;
        assert_eq!(refusal(small_float, 1).0, "RRS-E6010");
    }

    #[test]
    fn a_mismatch_names_both_sizes() {
        let header = r#"{"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 20]}}"#;
        let refused = check_safetensors_header(header.as_bytes(), 20).unwrap_err();
        assert_eq!(refused.code, "RRS-E6013");
        assert!(
            refused.message.contains("24 bytes") && refused.message.contains("20"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn tiling_problems_name_the_tensors_and_bytes() {
        let overlap = r#"{"a": {"dtype": "U8", "shape": [4], "data_offsets": [0, 4]},
            "b": {"dtype": "U8", "shape": [4], "data_offsets": [2, 6]}}"#;
        let refused = check_safetensors_header(overlap.as_bytes(), 6).unwrap_err();
        assert_eq!((refused.code.as_str(), refused.path.as_str()), ("RRS-E6014", "#/b"));
        assert!(refused.message.contains("b and a"), "{}", refused.message);
        let gap = r#"{"a": {"dtype": "U8", "shape": [2], "data_offsets": [0, 2]},
            "b": {"dtype": "U8", "shape": [2], "data_offsets": [4, 6]}}"#;
        let refused = check_safetensors_header(gap.as_bytes(), 6).unwrap_err();
        assert_eq!(refused.code, "RRS-E6015");
        assert!(refused.message.contains("byte 2"), "{}", refused.message);
        let trailing = r#"{"a": {"dtype": "U8", "shape": [2], "data_offsets": [0, 2]}}"#;
        let refused = check_safetensors_header(trailing.as_bytes(), 5).unwrap_err();
        assert_eq!(refused.code, "RRS-E6016");
        assert!(refused.message.contains("last 3 bytes"), "{}", refused.message);
        assert_eq!(refusal("{}", 1).0, "RRS-E6016");
    }

    #[test]
    fn metadata_maps_strings_to_strings_only() {
        assert_eq!(
            refusal(r#"{"__metadata__": {"step": 3}}"#, 0),
            pair("RRS-E6017", "#/__metadata__")
        );
        assert_eq!(refusal(r#"{"__metadata__": ["x"]}"#, 0).0, "RRS-E6017");
        assert_eq!(refusal(r#"{"__metadata__": {"a": null}}"#, 0).0, "RRS-E6017");
        let empty = check_safetensors_header(br#"{"__metadata__": {}}"#, 0).unwrap();
        assert!(empty.metadata.is_empty() && empty.entries.is_empty());
    }

    #[test]
    fn the_metadata_is_checked_before_the_tensors_and_tensors_by_name() {
        let header = r#"{"b": 1, "a": 2, "__metadata__": 3}"#;
        assert_eq!(refusal(header, 0).0, "RRS-E6017");
        let header = r#"{"b": 1, "a": 2}"#;
        assert_eq!(refusal(header, 0), pair("RRS-E6007", "#/a"));
    }
}
