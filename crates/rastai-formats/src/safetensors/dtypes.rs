//! The safetensors dtypes Rastai reads and writes, each with its name in a header and its element size in bytes.
//!
//! The list is the reference format's integer, float and bool types. A header naming any other dtype is refused
//! (`RRS-E6008`), so Rust and Python never disagree about what a file holds: Python maps each of these names to a
//! torch dtype (`rastai_models/io/safetensors_reader.py`) and never reads a name the wheel didn't accept.

/// The element types of a safetensors tensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetensorsDtype {
    F64,
    F32,
    F16,
    Bf16,
    F8E4m3,
    F8E5m2,
    I64,
    I32,
    I16,
    I8,
    U64,
    U32,
    U16,
    U8,
    Bool,
}

/// Every dtype with its header name and element size in bytes, in the reference format's order.
pub const SAFETENSORS_DTYPES: [(SafetensorsDtype, &str, u64); 15] = [
    (SafetensorsDtype::F64, "F64", 8),
    (SafetensorsDtype::F32, "F32", 4),
    (SafetensorsDtype::F16, "F16", 2),
    (SafetensorsDtype::Bf16, "BF16", 2),
    (SafetensorsDtype::F8E4m3, "F8_E4M3", 1),
    (SafetensorsDtype::F8E5m2, "F8_E5M2", 1),
    (SafetensorsDtype::I64, "I64", 8),
    (SafetensorsDtype::I32, "I32", 4),
    (SafetensorsDtype::I16, "I16", 2),
    (SafetensorsDtype::I8, "I8", 1),
    (SafetensorsDtype::U64, "U64", 8),
    (SafetensorsDtype::U32, "U32", 4),
    (SafetensorsDtype::U16, "U16", 2),
    (SafetensorsDtype::U8, "U8", 1),
    (SafetensorsDtype::Bool, "BOOL", 1),
];

/// The dtype a header names, or None for a name that isn't in the list. Names are case-sensitive, as in the
/// reference format: "f32" is not "F32".
pub fn dtype_from_name(name: &str) -> Option<SafetensorsDtype> {
    SAFETENSORS_DTYPES
        .iter()
        .find(|(_, row_name, _)| *row_name == name)
        .map(|(dtype, _, _)| *dtype)
}

/// The name a header writes for a dtype.
pub fn dtype_name(dtype: SafetensorsDtype) -> &'static str {
    match SAFETENSORS_DTYPES.iter().find(|(row_dtype, _, _)| *row_dtype == dtype) {
        Some((_, name, _)) => name,
        None => unreachable!("every SafetensorsDtype has a row in SAFETENSORS_DTYPES"),
    }
}

/// The size of one element in bytes.
pub fn dtype_element_bytes(dtype: SafetensorsDtype) -> u64 {
    match SAFETENSORS_DTYPES.iter().find(|(row_dtype, _, _)| *row_dtype == dtype) {
        Some((_, _, element_bytes)) => *element_bytes,
        None => unreachable!("every SafetensorsDtype has a row in SAFETENSORS_DTYPES"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_reads_back_to_its_own_dtype() {
        for (dtype, name, _) in SAFETENSORS_DTYPES {
            assert_eq!(dtype_from_name(name), Some(dtype));
            assert_eq!(dtype_name(dtype), name);
        }
    }

    #[test]
    fn the_list_is_the_reference_formats_names() {
        let names: Vec<&str> = SAFETENSORS_DTYPES.iter().map(|(_, name, _)| *name).collect();
        let expected = [
            "F64", "F32", "F16", "BF16", "F8_E4M3", "F8_E5M2", "I64", "I32", "I16", "I8", "U64", "U32", "U16", "U8",
            "BOOL",
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn element_sizes_match_the_type_widths() {
        assert_eq!(dtype_element_bytes(SafetensorsDtype::F64), 8);
        assert_eq!(dtype_element_bytes(SafetensorsDtype::Bf16), 2);
        assert_eq!(dtype_element_bytes(SafetensorsDtype::F8E4m3), 1);
        assert_eq!(dtype_element_bytes(SafetensorsDtype::U32), 4);
        assert_eq!(dtype_element_bytes(SafetensorsDtype::Bool), 1);
    }

    #[test]
    fn unknown_and_wrongly_cased_names_are_refused() {
        assert_eq!(dtype_from_name("F128"), None);
        assert_eq!(dtype_from_name("f32"), None);
        assert_eq!(dtype_from_name(""), None);
    }
}
