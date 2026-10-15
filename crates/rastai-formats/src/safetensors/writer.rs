//! The layout planner for writing a safetensors file (layout rust-control-plane.md 4.5; M2-13). It decides the header and
//! every tensor's offset. The caller writes the 8-byte little-endian length of `header_bytes`, then the header, then
//! each tensor's bytes in entry order, so the file is the layout exactly.
//!
//! The header is the one the Python writer writes (`rastai_models/io/safetensors_writer.py`): tensors sorted by name,
//! metadata first with its keys sorted, compact JSON, padded with spaces to 8 bytes. For ASCII names the bytes are the
//! same, and a planned layout reads back through `read_safetensors_header` with the same offsets. The tests read the file
//! the Python writer wrote (fixtures/safetensors-writer/) and check it against the plan.
//!
//! A refusal is `FormatsError::UnplannableLayout` (RRS-E1007): the caller asked for a layout that can't be written, so
//! nothing is written. The reader's RRS-E60xx codes are for files already on disk.

use crate::error::FormatsError;
use crate::safetensors::dtypes::{SafetensorsDtype, dtype_element_bytes, dtype_name};
use crate::safetensors::header_check::{MAX_HEADER_BYTES, METADATA_KEY, TensorEntry};

/// The header is padded to a multiple of this, so the data section starts aligned (as the Python writer does).
const HEADER_ALIGNMENT_BYTES: usize = 8;
const PADDING_BYTE: u8 = b' ';

/// One tensor to write: its name, dtype and shape. Its bytes come later, in the order the layout gives.
pub struct TensorPlan {
    pub name: String,
    pub dtype: SafetensorsDtype,
    pub shape: Vec<u64>,
}

/// A planned file. `header_bytes` is the JSON header with its padding, not the length prefix. `entries` are the tensors
/// in data-section order (sorted by name), each with its offsets from the start of the data section. The data section
/// is as long as the last entry's `end` (0 with no tensors).
pub struct SafetensorsLayout {
    pub header_bytes: Vec<u8>,
    pub entries: Vec<TensorEntry>,
}

/// Plans a file holding `tensors` and string `metadata`. Refused when two tensors share a name, a tensor is named like
/// the metadata key, a metadata key is given twice, a byte size overflows 64 bits, or the header would pass
/// `MAX_HEADER_BYTES`.
pub fn plan_safetensors(
    tensors: &[TensorPlan],
    metadata: &[(String, String)],
) -> Result<SafetensorsLayout, FormatsError> {
    let sorted = sorted_tensors(tensors)?;
    let sorted_metadata = sorted_metadata(metadata)?;
    let entries = place_tensors(&sorted)?;
    let header_bytes = build_header(&entries, &sorted_metadata);
    check_header_length(header_bytes.len() as u64)?;
    Ok(SafetensorsLayout { header_bytes, entries })
}

/// The reader refuses a header past `MAX_HEADER_BYTES`, so a plan it would refuse is refused before anything is written.
fn check_header_length(header_length: u64) -> Result<(), FormatsError> {
    if header_length > MAX_HEADER_BYTES {
        return Err(refused(format!(
            "the header would be {header_length} bytes, over the {MAX_HEADER_BYTES} byte limit"
        )));
    }
    Ok(())
}

/// The tensors in name order, the order their bytes are laid out in. A repeated name or the metadata key would make
/// the header hold a duplicate key or a tensor the reader takes for metadata.
fn sorted_tensors(tensors: &[TensorPlan]) -> Result<Vec<&TensorPlan>, FormatsError> {
    let mut sorted: Vec<&TensorPlan> = tensors.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    for (index, tensor) in sorted.iter().enumerate() {
        if tensor.name == METADATA_KEY {
            return Err(refused(format!(
                "a tensor can't be named {METADATA_KEY}: the header keeps that key for metadata"
            )));
        }
        if index > 0 && sorted[index - 1].name == tensor.name {
            return Err(refused(format!("two tensors are named {}", tensor.name)));
        }
    }
    Ok(sorted)
}

/// The metadata in key order, as the Python writer writes it. A repeated key is refused.
fn sorted_metadata(metadata: &[(String, String)]) -> Result<Vec<(String, String)>, FormatsError> {
    let mut sorted = metadata.to_vec();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in sorted.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(refused(format!("metadata key {} is given twice", pair[0].0)));
        }
    }
    Ok(sorted)
}

/// Each tensor's offsets, one after another in name order. Every size is computed with checked arithmetic, so a shape
/// that would wrap around is refused rather than given a small size.
fn place_tensors(sorted: &[&TensorPlan]) -> Result<Vec<TensorEntry>, FormatsError> {
    let mut entries = Vec::with_capacity(sorted.len());
    let mut offset: u64 = 0;
    for tensor in sorted {
        let size = tensor_byte_size(tensor)?;
        let end = offset.checked_add(size).ok_or_else(|| overflow(&tensor.name))?;
        entries.push(TensorEntry {
            name: tensor.name.clone(),
            dtype: tensor.dtype,
            shape: tensor.shape.clone(),
            start: offset,
            end,
        });
        offset = end;
    }
    Ok(entries)
}

/// The element size times every dimension, left to right, with checked multiplication.
fn tensor_byte_size(tensor: &TensorPlan) -> Result<u64, FormatsError> {
    let mut size = dtype_element_bytes(tensor.dtype);
    for dimension in &tensor.shape {
        size = size.checked_mul(*dimension).ok_or_else(|| overflow(&tensor.name))?;
    }
    Ok(size)
}

/// The JSON header: metadata first, then each tensor, with compact separators, padded with spaces to a multiple of 8.
fn build_header(entries: &[TensorEntry], metadata: &[(String, String)]) -> Vec<u8> {
    let mut members: Vec<String> = Vec::new();
    if !metadata.is_empty() {
        members.push(format!("{}:{}", json_text(METADATA_KEY), metadata_text(metadata)));
    }
    for entry in entries {
        members.push(format!("{}:{}", json_text(&entry.name), entry_text(entry)));
    }
    let mut header = format!("{{{}}}", members.join(",")).into_bytes();
    let padding = (HEADER_ALIGNMENT_BYTES - header.len() % HEADER_ALIGNMENT_BYTES) % HEADER_ALIGNMENT_BYTES;
    header.extend(std::iter::repeat_n(PADDING_BYTE, padding));
    header
}

fn entry_text(entry: &TensorEntry) -> String {
    format!(
        r#"{{"dtype":{},"shape":{},"data_offsets":[{},{}]}}"#,
        json_text(dtype_name(entry.dtype)),
        shape_text(&entry.shape),
        entry.start,
        entry.end
    )
}

fn metadata_text(metadata: &[(String, String)]) -> String {
    let members: Vec<String> = metadata
        .iter()
        .map(|(key, value)| format!("{}:{}", json_text(key), json_text(value)))
        .collect();
    format!("{{{}}}", members.join(","))
}

/// The dimensions as compact JSON: `[2,3]`, and `[]` for a scalar. The header and the model hash listing (8.6) both
/// write shapes this way, so there is one copy.
pub(crate) fn shape_text(shape: &[u64]) -> String {
    let sizes: Vec<String> = shape.iter().map(u64::to_string).collect();
    format!("[{}]", sizes.join(","))
}

/// A string as a JSON string literal. Writing a `Value` cannot fail, so this needs no error path.
fn json_text(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

fn overflow(tensor_name: &str) -> FormatsError {
    refused(format!("the byte size of tensor {tensor_name} overflows 64 bits"))
}

fn refused(reason: String) -> FormatsError {
    FormatsError::UnplannableLayout { reason }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::byte_source::SliceSource;
    use std::path::Path;

    use crate::safetensors::header_check::{HEADER_LENGTH_BYTES, MAX_HEADER_BYTES};
    use crate::safetensors::reader::read_safetensors_header;

    /// The header the Python writer's `build_header` gives for `sample_tensors` and `sample_metadata`, copied from
    /// its output: metadata first, then the tensors in name order.
    /// The 217-byte JSON is padded with 7 spaces to 224 bytes, so the padding bytes are part of the comparison.
    const PYTHON_HEADER: &[u8] = br#"{"__metadata__":{"format":"pt","step":"30"},"Head":{"dtype":"U8","shape":[0],"data_offsets":[0,0]},"bias":{"dtype":"BF16","shape":[3],"data_offsets":[0,6]},"weight":{"dtype":"F32","shape":[2,3],"data_offsets":[6,30]}}       "#;
    /// The file `write_safetensors` writes for the same tensors and metadata. The Python test
    /// (python/rastai_models/tests/test_safetensors_writer.py) checks the writer still writes these bytes, so the two
    /// layouts are kept equal by this file, not by a copy.
    const PYTHON_WRITTEN_FILE: &str = "../../fixtures/safetensors-writer/planned_layout.safetensors";
    /// The data section's length: the bias (6 bytes) and the weight (24 bytes).
    const SAMPLE_DATA_BYTES: u64 = 30;
    /// The Python writer's header for no tensors and no metadata: `{}` padded to 8 bytes.
    const PYTHON_EMPTY_HEADER: &[u8] = b"{}      ";

    fn tensor(name: &str, dtype: SafetensorsDtype, shape: &[u64]) -> TensorPlan {
        TensorPlan {
            name: name.to_string(),
            dtype,
            shape: shape.to_vec(),
        }
    }

    /// Given out of name order on purpose: the planner sorts them.
    fn sample_tensors() -> Vec<TensorPlan> {
        vec![
            tensor("weight", SafetensorsDtype::F32, &[2, 3]),
            tensor("bias", SafetensorsDtype::Bf16, &[3]),
            tensor("Head", SafetensorsDtype::U8, &[0]),
        ]
    }

    fn sample_metadata() -> Vec<(String, String)> {
        vec![
            ("step".to_string(), "30".to_string()),
            ("format".to_string(), "pt".to_string()),
        ]
    }

    /// The length prefix, the header and `data_length` zero bytes: a whole file as a writer lays it out.
    fn file_bytes(header: &[u8], data_length: usize) -> Vec<u8> {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header);
        bytes.extend(std::iter::repeat_n(0u8, data_length));
        bytes
    }

    fn refusal_code(tensors: &[TensorPlan], metadata: &[(String, String)]) -> &'static str {
        match plan_safetensors(tensors, metadata) {
            Ok(_) => panic!("the plan should be refused"),
            Err(error) => error.code(),
        }
    }

    #[test]
    fn the_header_is_the_python_writers_header_byte_for_byte() {
        let layout = plan_safetensors(&sample_tensors(), &sample_metadata()).unwrap();
        assert_eq!(layout.header_bytes, PYTHON_HEADER);
        let names: Vec<&str> = layout.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Head", "bias", "weight"]);
        let offsets: Vec<(u64, u64)> = layout.entries.iter().map(|entry| (entry.start, entry.end)).collect();
        assert_eq!(offsets, [(0, 0), (0, 6), (6, 30)]);
    }

    #[test]
    fn shape_text_is_compact_json() {
        assert_eq!(shape_text(&[2, 3]), "[2,3]");
        assert_eq!(shape_text(&[]), "[]");
    }

    #[test]
    fn an_empty_plan_is_the_python_writers_empty_header_padded_to_eight_bytes() {
        let layout = plan_safetensors(&[], &[]).unwrap();
        assert_eq!(layout.header_bytes, PYTHON_EMPTY_HEADER);
        assert!(layout.entries.is_empty());
    }

    #[test]
    fn a_planned_layout_reads_back_with_the_same_offsets() {
        let layout = plan_safetensors(&sample_tensors(), &sample_metadata()).unwrap();
        let bytes = file_bytes(&layout.header_bytes, 30);
        let header = read_safetensors_header(&SliceSource { bytes: &bytes }).unwrap();
        assert_eq!(header.entries, layout.entries);
        assert_eq!(
            header.metadata,
            [
                ("format".to_string(), "pt".to_string()),
                ("step".to_string(), "30".to_string())
            ]
        );
        assert_eq!(header.data_start, HEADER_LENGTH_BYTES + PYTHON_HEADER.len() as u64);
    }

    #[test]
    fn an_empty_layout_reads_back_with_no_tensors() {
        let layout = plan_safetensors(&[], &[]).unwrap();
        let bytes = file_bytes(&layout.header_bytes, 0);
        let header = read_safetensors_header(&SliceSource { bytes: &bytes }).unwrap();
        assert!(header.entries.is_empty() && header.metadata.is_empty());
    }

    #[test]
    fn offsets_are_contiguous_in_name_order() {
        let layout = plan_safetensors(&sample_tensors(), &[]).unwrap();
        let mut expected_start = 0;
        for entry in &layout.entries {
            assert_eq!(entry.start, expected_start, "{}", entry.name);
            expected_start = entry.end;
        }
    }

    #[test]
    fn two_tensors_with_one_name_are_refused() {
        let tensors = [
            tensor("a", SafetensorsDtype::U8, &[1]),
            tensor("a", SafetensorsDtype::U8, &[2]),
        ];
        assert_eq!(refusal_code(&tensors, &[]), "RRS-E1007");
    }

    #[test]
    fn a_tensor_named_like_the_metadata_key_is_refused() {
        let tensors = [tensor(METADATA_KEY, SafetensorsDtype::U8, &[1])];
        assert_eq!(refusal_code(&tensors, &[]), "RRS-E1007");
    }

    #[test]
    fn a_metadata_key_given_twice_is_refused() {
        let metadata = [("k".to_string(), "1".to_string()), ("k".to_string(), "2".to_string())];
        assert_eq!(refusal_code(&[], &metadata), "RRS-E1007");
    }

    #[test]
    fn a_shape_that_would_wrap_is_refused_and_not_given_a_small_size() {
        let tensors = [tensor("big", SafetensorsDtype::U8, &[4_294_967_296, 4_294_967_296])];
        let error = plan_safetensors(&tensors, &[]).err().unwrap();
        match error {
            FormatsError::UnplannableLayout { reason } => assert!(reason.contains("big"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_header_past_the_cap_is_refused_and_one_at_the_cap_is_not() {
        assert!(check_header_length(MAX_HEADER_BYTES).is_ok());
        let refused = check_header_length(MAX_HEADER_BYTES + 1).unwrap_err();
        assert_eq!(refused.code(), "RRS-E1007");
        let FormatsError::UnplannableLayout { reason } = refused else {
            panic!("a header past the cap is not a layout that can be planned");
        };
        assert!(reason.contains("104857600 byte limit"), "{reason}");
    }

    #[test]
    fn the_planned_layout_is_the_layout_the_python_writer_wrote() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PYTHON_WRITTEN_FILE);
        let bytes = std::fs::read(path).unwrap();
        let header = read_safetensors_header(&SliceSource { bytes: &bytes }).unwrap();
        let layout = plan_safetensors(&sample_tensors(), &sample_metadata()).unwrap();
        assert_eq!(header.entries, layout.entries);
        assert_eq!(
            header.data_start,
            HEADER_LENGTH_BYTES + layout.header_bytes.len() as u64
        );
        let header_end = (HEADER_LENGTH_BYTES as usize) + layout.header_bytes.len();
        assert_eq!(
            &bytes[HEADER_LENGTH_BYTES as usize..header_end],
            layout.header_bytes.as_slice()
        );
        assert_eq!(bytes.len() as u64, header.data_start + SAMPLE_DATA_BYTES);
    }

    #[test]
    fn a_total_that_overflows_is_refused_even_when_each_tensor_fits() {
        let half = 1u64 << 63;
        let tensors = [
            tensor("a", SafetensorsDtype::U8, &[half]),
            tensor("b", SafetensorsDtype::U8, &[half]),
        ];
        assert_eq!(refusal_code(&tensors, &[]), "RRS-E1007");
    }
}
