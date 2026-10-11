//! The stable pretty writer for files on disk: two-space indent, one field per line, a newline at the end, and
//! fields in the order the struct declares them (the description order, layout 4.2). The same value always gives
//! the same bytes, so saving an unchanged document changes nothing in git.
//!
//! serde_json does the writing (contracts 2.5: it holds the values and writes the pretty form); this file fixes
//! the settings. Map keys come out sorted, because serde_json's maps are sorted. Content hashes never use this
//! form; they use canonical.rs.

use serde::Serialize;

use crate::error::FormatsError;

const INDENT: &[u8] = b"  ";
const FINAL_NEWLINE: u8 = b'\n';

/// Writes any serializable value (a document struct or a `serde_json::Value`) as pretty JSON text.
/// Fails only for a value JSON can't hold, such as a map with non-string keys.
pub fn pretty_json<T: Serialize>(value: &T) -> Result<Vec<u8>, FormatsError> {
    let mut output = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(INDENT);
    let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
    value
        .serialize(&mut serializer)
        .map_err(|error| FormatsError::NotSerializable {
            reason: error.to_string(),
        })?;
    output.push(FINAL_NEWLINE);
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::json::reader::read_json;
    use crate::limits::ImportLimits;
    use serde_json::json;
    use std::collections::BTreeMap;

    // Field order here is deliberately not alphabetical: the writer must keep it.
    #[derive(Serialize)]
    struct Sample {
        type_name: String,
        schema_version: u32,
        tags: Vec<String>,
        empty: Vec<u32>,
        threshold: f64,
    }

    #[test]
    fn struct_fields_keep_their_order_with_two_space_indent() {
        let sample = Sample {
            type_name: "rastai.eval".to_string(),
            schema_version: 1,
            tags: vec!["a".to_string(), "b".to_string()],
            empty: Vec::new(),
            threshold: 0.5,
        };
        let expected = "{\n  \"type_name\": \"rastai.eval\",\n  \"schema_version\": 1,\n  \"tags\": [\n    \"a\",\n    \
                        \"b\"\n  ],\n  \"empty\": [],\n  \"threshold\": 0.5\n}\n";
        assert_eq!(String::from_utf8(pretty_json(&sample).unwrap()).unwrap(), expected);
    }

    #[test]
    fn same_value_same_bytes_and_it_reads_back_equal() {
        let value = json!({"z": [1, 2.5, -3e-7], "a": {"nested": null, "text": "é\n"}, "m": {}});
        let first = pretty_json(&value).unwrap();
        assert_eq!(first, pretty_json(&value).unwrap());
        let parsed = read_json(&first, &ImportLimits::default()).unwrap();
        assert_eq!(parsed.value, value);
        assert!(first.ends_with(b"}\n"));
    }

    #[test]
    fn map_keys_that_are_not_strings_are_refused() {
        let mut odd_keys: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
        odd_keys.insert(vec![1], 1);
        let error = pretty_json(&odd_keys).unwrap_err();
        assert_eq!(error.code(), "RRS-E1002");
    }
}
