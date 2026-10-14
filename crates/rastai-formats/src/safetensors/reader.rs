//! The strict safetensors reader over a `ByteSource` (layout rust-control-plane.md 4.5; M2-13). It reads the 8-byte
//! length, then the header, and hands the header to `check_safetensors_header`, which decides every refusal. Tensor
//! bytes are read only after that, one tensor at a time into the caller's buffer, so a large file is never held in
//! memory. Nothing here maps the file: the daemon and the node read headers and stream tensors.

// A refusal is a `Diagnostic`, the contract type every caller renders (layout 4.5), as in header_check.rs. A refused
// file is rare, so it is returned as it is, not boxed.
#![allow(clippy::result_large_err)]

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};

use crate::byte_source::ByteSource;
use crate::error::FormatsError;
use crate::safetensors::header_check::{
    HEADER_LENGTH_BYTES, SafetensorsHeader, TensorEntry, check_safetensors_header, read_safetensors_header_length,
};

// Catalog codes (contracts 10.3) this file reports. The header check owns the other RRS-E60xx codes.
const CODE_OFFSETS_OUTSIDE: &str = "RRS-E6011";
const CODE_SOURCE_NOT_READABLE: &str = "RRS-D3001";
const READ_FAILED_REASON: &str = "the source could not be read";

/// Reads and checks a whole header from a source. Every refusal is a `Diagnostic` with the code of the first problem
/// found (the length, then the header rules of `check_safetensors_header`). A file shorter than the length prefix is
/// refused with `RRS-E6001` before the prefix is read in full.
pub fn read_safetensors_header(source: &dyn ByteSource) -> Result<SafetensorsHeader, Diagnostic> {
    let file_length = source.length();
    let prefix = read_bytes(source, 0, prefix_length(file_length))?;
    let header_length = read_safetensors_header_length(&prefix, file_length)?;
    // header_length is at most MAX_HEADER_BYTES here, so it fits in memory on every target.
    let header_bytes = read_bytes(source, HEADER_LENGTH_BYTES, header_length as usize)?;
    let data_length = file_length - HEADER_LENGTH_BYTES - header_length;
    check_safetensors_header(&header_bytes, data_length)
}

/// Reads one tensor's bytes into `buffer`, replacing what it held. The header was accepted against the data section,
/// but the source is checked again here: a source shorter than the header claims is refused with `RRS-E6011` before
/// the buffer grows to the claimed size.
pub fn read_tensor_bytes(
    source: &dyn ByteSource,
    header: &SafetensorsHeader,
    entry: &TensorEntry,
    buffer: &mut Vec<u8>,
) -> Result<(), Diagnostic> {
    let outside = || outside_diagnostic(&entry.name);
    let offset = header.data_start.checked_add(entry.start).ok_or_else(outside)?;
    let length = entry.end.checked_sub(entry.start).ok_or_else(outside)?;
    let end = offset.checked_add(length).ok_or_else(outside)?;
    if end > source.length() {
        return Err(outside());
    }
    let length = usize::try_from(length).map_err(|_| outside())?;
    buffer.clear();
    buffer.resize(length, 0);
    source.read_at(offset, buffer).map_err(read_diagnostic)
}

/// The bytes the length prefix needs: all of them, or everything the file has when it is shorter (the header check
/// then refuses the file as too short).
fn prefix_length(file_length: u64) -> usize {
    file_length.min(HEADER_LENGTH_BYTES) as usize
}

/// `length` bytes at `offset`, in one read. The caller has checked `length` against the file, so the buffer is no
/// bigger than the file.
fn read_bytes(source: &dyn ByteSource, offset: u64, length: usize) -> Result<Vec<u8>, Diagnostic> {
    let mut buffer = vec![0u8; length];
    source.read_at(offset, &mut buffer).map_err(read_diagnostic)?;
    Ok(buffer)
}

fn read_diagnostic(error: FormatsError) -> Diagnostic {
    let reason = match error {
        FormatsError::ReadFailed { reason, .. } => reason,
        _ => READ_FAILED_REASON.to_string(),
    };
    diagnostic_at(CODE_SOURCE_NOT_READABLE, &[("reason", &reason)], "")
}

fn outside_diagnostic(tensor_name: &str) -> Diagnostic {
    diagnostic_at(CODE_OFFSETS_OUTSIDE, &[("tensor_name", tensor_name)], "")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::byte_source::SliceSource;
    use crate::safetensors::dtypes::SafetensorsDtype;

    const TWO_TENSOR_HEADER: &str = r#"{"__metadata__": {"format": "pt"},
        "a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 24]},
        "b": {"dtype": "U8", "shape": [8], "data_offsets": [24, 32]}}"#;

    /// The length prefix, the header and then `data` bytes: a whole file.
    fn file_with(header: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    fn code_of(bytes: &[u8]) -> String {
        read_safetensors_header(&SliceSource { bytes }).unwrap_err().code
    }

    #[test]
    fn a_valid_file_reads_the_same_header_as_the_check_alone() {
        let data: Vec<u8> = (0..32).collect();
        let bytes = file_with(TWO_TENSOR_HEADER, &data);
        let read = read_safetensors_header(&SliceSource { bytes: &bytes }).unwrap();
        let checked = check_safetensors_header(TWO_TENSOR_HEADER.as_bytes(), 32).unwrap();
        assert_eq!(read, checked);
        assert_eq!(read.entries[0].dtype, SafetensorsDtype::F32);
        assert_eq!(read.data_start, HEADER_LENGTH_BYTES + TWO_TENSOR_HEADER.len() as u64);
    }

    #[test]
    fn a_file_shorter_than_the_prefix_is_refused_as_short() {
        assert_eq!(code_of(&[]), "RRS-E6001");
        assert_eq!(code_of(&[1, 0, 0, 0, 0, 0, 0]), "RRS-E6001");
    }

    #[test]
    fn a_length_past_the_end_is_refused_before_the_header_is_read() {
        let bytes = 100u64.to_le_bytes();
        assert_eq!(code_of(&bytes), "RRS-E6002");
    }

    #[test]
    fn a_tensor_reads_its_own_bytes_after_the_header() {
        let data: Vec<u8> = (0..32).collect();
        let bytes = file_with(TWO_TENSOR_HEADER, &data);
        let source = SliceSource { bytes: &bytes };
        let header = read_safetensors_header(&source).unwrap();
        let mut buffer = Vec::new();
        read_tensor_bytes(&source, &header, &header.entries[1], &mut buffer).unwrap();
        assert_eq!(buffer, data[24..32]);
        read_tensor_bytes(&source, &header, &header.entries[0], &mut buffer).unwrap();
        assert_eq!(buffer, data[0..24]);
    }

    #[test]
    fn a_tensor_past_a_shorter_source_is_refused_before_the_buffer_grows() {
        let data: Vec<u8> = (0..32).collect();
        let full = file_with(TWO_TENSOR_HEADER, &data);
        let header = check_safetensors_header(TWO_TENSOR_HEADER.as_bytes(), 32).unwrap();
        // Only 8 of the 32 data bytes are there: the first tensor (24 bytes) runs past the source.
        let short = &full[..full.len() - 24];
        let mut buffer = Vec::new();
        let refused =
            read_tensor_bytes(&SliceSource { bytes: short }, &header, &header.entries[0], &mut buffer).unwrap_err();
        assert_eq!(refused.code, "RRS-E6011");
        assert!(refused.message.contains("tensor a "), "{}", refused.message);
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_shape_that_would_wrap_is_refused_with_its_overflow_code() {
        let header = r#"{"a": {"dtype": "U8", "shape": [4294967296, 4294967296], "data_offsets": [0, 0]}}"#;
        let bytes = file_with(header, &[]);
        assert_eq!(code_of(&bytes), "RRS-E6012");
    }
}
