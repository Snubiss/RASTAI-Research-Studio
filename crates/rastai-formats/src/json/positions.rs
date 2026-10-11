//! Where each value of a JSON document starts in the text, keyed by its JSON pointer. The reader fills the map, and
//! the validation layers look positions up so every diagnostic can say `file:line:column#/pointer`.
//!
//! Lines and columns start at 1. A column counts characters (Unicode scalar values), not bytes, so it matches the
//! `Diagnostic.path` grammar (rastai-diag path.rs) and Python's `json` error columns.

use std::collections::BTreeMap;

/// The pointer of the whole document (RFC 6901: the empty string).
const ROOT_POINTER: &str = "";

/// A line and column in the source text, both starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

/// JSON pointer → where that value starts. An object member is recorded at its key's opening quote, because a
/// diagnostic about a field (unknown, wrong kind, duplicate) points at the field's name. An array element and the
/// root are recorded at the first character of the value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PositionMap {
    positions: BTreeMap<String, TextPosition>,
}

/// Records where the value at `pointer` starts. A later record for the same pointer replaces the earlier one.
pub(crate) fn record_position(map: &mut PositionMap, pointer: &str, position: TextPosition) {
    map.positions.insert(pointer.to_string(), position);
}

/// The position of exactly this pointer, if the reader saw a value there.
pub fn position_of(map: &PositionMap, pointer: &str) -> Option<TextPosition> {
    map.positions.get(pointer).copied()
}

/// The position of this pointer or, when it has none, of its nearest recorded parent. A missing required field
/// has no position of its own, so its diagnostic points at the object that should hold it.
pub fn find_position(map: &PositionMap, pointer: &str) -> Option<TextPosition> {
    let mut current = pointer;
    loop {
        if let Some(position) = position_of(map, current) {
            return Some(position);
        }
        if current == ROOT_POINTER {
            return None;
        }
        current = parent_pointer(current);
    }
}

/// The number of recorded positions; one per value the reader saw. Tests use it to check nothing is recorded twice.
#[cfg(test)]
pub(crate) fn position_count(map: &PositionMap) -> usize {
    map.positions.len()
}

/// `/a/b/0` → `/a/b`, `/a` → the root. Splitting at the last `/` is safe on escaped tokens: `~1` stands for `/`.
fn parent_pointer(pointer: &str) -> &str {
    match pointer.rfind('/') {
        Some(index) => &pointer[..index],
        None => ROOT_POINTER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, column: u32) -> TextPosition {
        TextPosition { line, column }
    }

    #[test]
    fn exact_lookup_and_parent_fallback() {
        let mut map = PositionMap::default();
        record_position(&mut map, "", at(1, 1));
        record_position(&mut map, "/inputs", at(2, 3));
        record_position(&mut map, "/inputs/0", at(3, 5));
        assert_eq!(position_of(&map, "/inputs/0"), Some(at(3, 5)));
        assert_eq!(position_of(&map, "/inputs/0/range"), None);
        assert_eq!(find_position(&map, "/inputs/0/range"), Some(at(3, 5)));
        assert_eq!(find_position(&map, "/missing/deep/field"), Some(at(1, 1)));
        assert_eq!(position_count(&map), 3);
    }

    #[test]
    fn empty_map_has_no_positions() {
        let map = PositionMap::default();
        assert_eq!(find_position(&map, "/a"), None);
        assert_eq!(find_position(&map, ""), None);
    }

    #[test]
    fn escaped_slash_stays_inside_its_token() {
        let mut map = PositionMap::default();
        record_position(&mut map, "/a~1b", at(4, 2));
        assert_eq!(find_position(&map, "/a~1b/c"), Some(at(4, 2)));
    }
}
