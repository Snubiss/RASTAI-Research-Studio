//! Strict safetensors reader: the same contract as the Python reader in src/rastai_spike/safetensors_reader.py.
//!
//!   - the header is a UTF-8 JSON object with no duplicate keys, at most 100 MB;
//!   - "__metadata__", if present, maps strings to strings;
//!   - every other entry has a known dtype, a shape of non-negative integers, and integer data_offsets
//!     [start, end] whose length equals the shape's byte size, computed without overflow;
//!   - the tensors tile the data section exactly: contiguous, no overlaps, no gaps, nothing left over.

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::path::Path;
use std::time::Instant;

use memmap2::Mmap;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::{Value, json};

const HEADER_LENGTH_BYTES: usize = 8;
const MAX_HEADER_BYTES: u64 = 100 * 1024 * 1024;
const METADATA_KEY: &str = "__metadata__";

pub struct TensorEntry {
    pub name: String,
    pub start: u64,
    pub end: u64,
}

pub struct SafetensorsFile {
    mapping: Mmap,
    data_start: usize,
    pub entries: Vec<TensorEntry>, // sorted by position in the data section
}

impl SafetensorsFile {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|error| format!("cannot open: {error}"))?;
        let file_length = file.metadata().map_err(|error| error.to_string())?.len();
        if file_length < HEADER_LENGTH_BYTES as u64 {
            return Err("file is shorter than the 8-byte header length".into());
        }
        // Safety: the file is opened read-only and not modified while mapped.
        let mapping = unsafe { Mmap::map(&file) }.map_err(|error| format!("cannot map: {error}"))?;

        let header_length = read_header_length(&mapping)?;
        let data_start = HEADER_LENGTH_BYTES + header_length;
        let header_text = std::str::from_utf8(&mapping[HEADER_LENGTH_BYTES..data_start])
            .map_err(|_| "header is not valid UTF-8".to_string())?;
        let pairs = parse_object_rejecting_duplicates(header_text)?;
        let data_length = (mapping.len() - data_start) as u64;
        let entries = build_entries(pairs, data_length)?;
        Ok(Self { mapping, data_start, entries })
    }

    pub fn tensor_bytes(&self, entry: &TensorEntry) -> &[u8] {
        let start = self.data_start + entry.start as usize;
        let end = self.data_start + entry.end as usize;
        &self.mapping[start..end]
    }
}

fn read_header_length(mapping: &[u8]) -> Result<usize, String> {
    let header_length = u64::from_le_bytes(mapping[..HEADER_LENGTH_BYTES].try_into().unwrap());
    if header_length > MAX_HEADER_BYTES {
        return Err(format!("header is {header_length} bytes, over the {MAX_HEADER_BYTES} byte limit"));
    }
    if HEADER_LENGTH_BYTES as u64 + header_length > mapping.len() as u64 {
        return Err("header length points past the end of the file".into());
    }
    Ok(header_length as usize)
}

// ---- JSON: a top-level object whose duplicate keys are an error, not "last one wins" ----------------

struct UniqueKeyObject(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for UniqueKeyObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UniqueKeyVisitor)
    }
}

struct UniqueKeyVisitor;

impl<'de> Visitor<'de> for UniqueKeyVisitor {
    type Value = UniqueKeyObject;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = HashSet::new();
        let mut pairs = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            if !seen.insert(key.clone()) {
                return Err(serde::de::Error::custom("header has duplicate keys"));
            }
            pairs.push((key, value));
        }
        Ok(UniqueKeyObject(pairs))
    }
}

fn parse_object_rejecting_duplicates(text: &str) -> Result<Vec<(String, Value)>, String> {
    serde_json::from_str::<UniqueKeyObject>(text)
        .map(|object| object.0)
        .map_err(|error| format!("header is not a valid JSON object: {error}"))
}

// ---- Entries ---------------------------------------------------------------------------------------

fn bytes_per_element(dtype: &str) -> Option<u64> {
    match dtype {
        "F64" | "I64" | "U64" => Some(8),
        "F32" | "I32" | "U32" => Some(4),
        "F16" | "BF16" | "I16" | "U16" => Some(2),
        "F8_E4M3" | "F8_E5M2" | "I8" | "U8" | "BOOL" => Some(1),
        _ => None,
    }
}

/// A JSON integer that fits in u64. Floats, negatives, booleans and huge numbers are all rejected.
fn plain_unsigned(value: &Value) -> Option<u64> {
    value.as_u64()
}

fn build_entries(pairs: Vec<(String, Value)>, data_length: u64) -> Result<Vec<TensorEntry>, String> {
    let mut entries = Vec::new();
    for (name, description) in pairs {
        if name == METADATA_KEY {
            check_metadata(&description)?;
        } else {
            entries.push(parse_entry(name, &description, data_length)?);
        }
    }
    entries.sort_by_key(|entry| (entry.start, entry.end));
    check_exact_tiling(&entries, data_length)?;
    Ok(entries)
}

fn check_metadata(metadata: &Value) -> Result<(), String> {
    match metadata.as_object() {
        Some(object) if object.values().all(Value::is_string) => Ok(()),
        _ => Err("__metadata__ must map strings to strings".into()),
    }
}

fn parse_entry(name: String, description: &Value, data_length: u64) -> Result<TensorEntry, String> {
    let object = description.as_object().ok_or_else(|| format!("{name}: entry must be an object"))?;

    let dtype = object.get("dtype").and_then(Value::as_str).unwrap_or("");
    let element_size = bytes_per_element(dtype).ok_or_else(|| format!("{name}: unsupported dtype {dtype:?}"))?;

    let shape: Vec<u64> = object
        .get("shape")
        .and_then(Value::as_array)
        .and_then(|sizes| sizes.iter().map(plain_unsigned).collect())
        .ok_or_else(|| format!("{name}: shape must be a list of non-negative integers"))?;

    let offsets: Vec<u64> = object
        .get("data_offsets")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 2)
        .and_then(|values| values.iter().map(plain_unsigned).collect())
        .ok_or_else(|| format!("{name}: data_offsets must be [start, end]"))?;
    let (start, end) = (offsets[0], offsets[1]);
    if !(start <= end && end <= data_length) {
        return Err(format!("{name}: data_offsets [{start}, {end}] fall outside the data section"));
    }

    // checked_mul: a shape like [2^32, 2^32] must not wrap around to 0 and "match" empty offsets.
    let expected_bytes = shape
        .iter()
        .try_fold(element_size, |total, &size| total.checked_mul(size))
        .ok_or_else(|| format!("{name}: shape's byte size overflows 64 bits"))?;
    if end - start != expected_bytes {
        return Err(format!("{name}: shape needs {expected_bytes} bytes but offsets cover {}", end - start));
    }
    Ok(TensorEntry { name, start, end })
}

fn check_exact_tiling(entries: &[TensorEntry], data_length: u64) -> Result<(), String> {
    let mut expected_start = 0;
    for entry in entries {
        if entry.start < expected_start {
            return Err(format!("{} overlaps the tensor before it", entry.name));
        }
        if entry.start > expected_start {
            return Err(format!("gap in the data section before {}", entry.name));
        }
        expected_start = entry.end;
    }
    if expected_start != data_length {
        return Err(format!("{} bytes at the end of the data section belong to no tensor", data_length - expected_start));
    }
    Ok(())
}

// ---- Commands ----------------------------------------------------------------------------------------

pub fn read_command(arguments: &[String]) -> Result<(), String> {
    let path = Path::new(crate::argument(arguments, 0, "file")?);

    let header_start = Instant::now();
    let file = SafetensorsFile::open(path)?;
    let header_milliseconds = header_start.elapsed().as_secs_f64() * 1000.0;

    let read_start = Instant::now();
    // CRC32C (the "iSCSI" polynomial), with the SIMD folding that matches the JDK's built-in CRC32C.
    let mut checksum = crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc32Iscsi);
    let mut total_bytes = 0u64;
    for entry in &file.entries {
        let bytes = file.tensor_bytes(entry);
        checksum.update(bytes);
        total_bytes += bytes.len() as u64;
    }
    let checksum = checksum.finalize();
    let read_milliseconds = read_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "{}",
        json!({
            "tensors": file.entries.len(),
            "bytes": total_bytes,
            "crc32c": checksum,
            "header_ms": header_milliseconds,
            "read_ms": read_milliseconds,
        })
    );
    Ok(())
}

pub fn validate_command(arguments: &[String]) -> Result<(), String> {
    let folder = Path::new(crate::argument(arguments, 0, "folder")?);
    let mut paths: Vec<_> = std::fs::read_dir(folder)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();

    let mut verdicts = serde_json::Map::new();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let verdict = match SafetensorsFile::open(&path) {
            Ok(file) => json!({"accepted": true, "tensors": file.entries.len()}),
            Err(error) => json!({"accepted": false, "error": error}),
        };
        verdicts.insert(name, verdict);
    }
    println!("{}", Value::Object(verdicts));
    Ok(())
}
