//! Where each value of a JSON document starts in the text. The reader fills the map, and the validation layers
//! look positions up by JSON pointer so every diagnostic can say `file:line:column#/pointer`.
//!
//! Lines and columns start at 1. A column counts characters (Unicode scalar values), not bytes, so it matches the
//! `Diagnostic.path` grammar (rastai-diag path.rs) and Python's `json` error columns.
//!
//! The map is a tree shaped like the document: each value's node holds its position and its children by key or
//! index. A map keyed by full pointer strings stored every parent's path again in every child, so memory grew with
//! depth times key length; a hostile 16 MiB document with long keys 64 levels deep used over a gigabyte. In the
//! tree each key is stored once, and memory is linear in the document (ADR-0032 decision 8).
//!
//! An object's members are a plain list sorted by key, found by binary search. A BTreeMap per object allocated a
//! whole node of 11 slots even for one member, so a document of one-member objects took twice what serde_json's own
//! tree takes (1.42 GiB for `[{"a":0},{"a":0},...]` at 8 MB); the list holds just its members (limits.rs has the
//! measured costs).

use rastai_diag::path::{DiagnosticPath, format_path};

use crate::json::pointer::{ROOT_POINTER, parse_array_index, unescape_pointer_token};

const POINTER_SEPARATOR: char = '/';

/// A line and column in the source text, both starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

/// Where each value of a document starts. An object member is recorded at its key's opening quote, because a
/// diagnostic about a field (unknown, wrong kind, duplicate) points at the field's name. An array element and the
/// root are recorded at the first character of the value. Empty until the reader fills it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PositionMap {
    root: Option<PositionNode>,
}

/// One value: where it starts, and where its members or elements start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PositionNode {
    pub(crate) position: TextPosition,
    pub(crate) children: PositionChildren,
}

/// The children of a value, by the same keys or indexes as the value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PositionChildren {
    /// A string, number, `true`, `false` or `null`.
    Leaf,
    /// An object's members, sorted by unescaped key. Keys are unique: the reader refuses a repeated one.
    Members(Vec<MemberPosition>),
    /// An array's elements, in order.
    Elements(Vec<PositionNode>),
}

/// One object member: its unescaped key and its value's node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberPosition {
    pub(crate) key: String,
    pub(crate) node: PositionNode,
}

/// How far a pointer got down the tree.
struct PointerWalk {
    /// The position of the deepest value the walk reached; None only when the map is empty.
    deepest: Option<TextPosition>,
    /// True when every token of the pointer was found.
    complete: bool,
}

/// An object's member list ready for lookup: sorted by key, and trimmed to its length so no spare capacity stays
/// allocated for the life of the map.
pub(crate) fn finish_members(mut members: Vec<MemberPosition>) -> PositionChildren {
    members.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    members.shrink_to_fit();
    PositionChildren::Members(members)
}

/// An array's element list, trimmed to its length like `finish_members`.
pub(crate) fn finish_elements(mut elements: Vec<PositionNode>) -> PositionChildren {
    elements.shrink_to_fit();
    PositionChildren::Elements(elements)
}

/// The map of a whole document, from the root value's node.
pub(crate) fn position_map_from_root(root: PositionNode) -> PositionMap {
    PositionMap { root: Some(root) }
}

/// The position of exactly this pointer, if the reader saw a value there.
pub fn position_of(map: &PositionMap, pointer: &str) -> Option<TextPosition> {
    let walk = walk_pointer(map, pointer);
    if walk.complete { walk.deepest } else { None }
}

/// The position of this pointer or, when it has none, of its nearest recorded parent. A missing required field
/// has no position of its own, so its diagnostic points at the object that should hold it.
pub fn find_position(map: &PositionMap, pointer: &str) -> Option<TextPosition> {
    walk_pointer(map, pointer).deepest
}

/// `file:line:column#pointer` for a value in the document. The position is the pointer's own or, for a missing
/// field, its nearest recorded parent's (the object that should hold it).
pub(crate) fn diagnostic_path(file_name: &str, positions: &PositionMap, pointer: &str) -> String {
    let position = find_position(positions, pointer);
    format_path(&DiagnosticPath {
        file: file_name.to_string(),
        line: position.map_or(0, |place| place.line),
        column: position.map_or(0, |place| place.column),
        json_pointer: pointer.to_string(),
    })
}

/// The number of recorded positions; one per value the reader saw. Tests use it to check nothing is recorded twice.
#[cfg(test)]
pub(crate) fn position_count(map: &PositionMap) -> usize {
    match &map.root {
        Some(root) => count_nodes(root),
        None => 0,
    }
}

#[cfg(test)]
fn count_nodes(node: &PositionNode) -> usize {
    let children_count: usize = match &node.children {
        PositionChildren::Leaf => 0,
        PositionChildren::Members(members) => members.iter().map(|member| count_nodes(&member.node)).sum(),
        PositionChildren::Elements(elements) => elements.iter().map(count_nodes).sum(),
    };
    1 + children_count
}

/// Follows the pointer's tokens down from the root, stopping at the first one that isn't there. Text that isn't a
/// pointer (no leading `/`, or a bad `~` escape) stops the walk where it goes wrong, so `find_position` still
/// falls back to the nearest parent, as it does for a missing field.
fn walk_pointer(map: &PositionMap, pointer: &str) -> PointerWalk {
    let Some(root) = &map.root else {
        return PointerWalk {
            deepest: None,
            complete: false,
        };
    };
    let mut walk = PointerWalk {
        deepest: Some(root.position),
        complete: pointer == ROOT_POINTER,
    };
    let Some(tokens) = pointer.strip_prefix(POINTER_SEPARATOR) else {
        return walk;
    };
    let mut node = root;
    for escaped_token in tokens.split(POINTER_SEPARATOR) {
        let Some(child) = find_child(node, escaped_token) else {
            return walk;
        };
        node = child;
        walk.deepest = Some(child.position);
    }
    walk.complete = true;
    walk
}

/// One step down: the member with this key or the element at this index.
fn find_child<'a>(node: &'a PositionNode, escaped_token: &str) -> Option<&'a PositionNode> {
    let token = unescape_pointer_token(escaped_token)?;
    match &node.children {
        PositionChildren::Leaf => None,
        PositionChildren::Members(members) => find_member(members, &token),
        PositionChildren::Elements(elements) => elements.get(parse_array_index(&token)?),
    }
}

/// The member with this key, by binary search over the key-sorted list.
fn find_member<'a>(members: &'a [MemberPosition], key: &str) -> Option<&'a PositionNode> {
    let index = members.binary_search_by(|member| member.key.as_str().cmp(key)).ok()?;
    Some(&members[index].node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, column: u32) -> TextPosition {
        TextPosition { line, column }
    }

    fn leaf(position: TextPosition) -> PositionNode {
        PositionNode {
            position,
            children: PositionChildren::Leaf,
        }
    }

    // Members are given in any order, as in a document; finish_members sorts them.
    fn object(position: TextPosition, members: Vec<(&str, PositionNode)>) -> PositionNode {
        let mut children = Vec::new();
        for (key, node) in members {
            children.push(MemberPosition {
                key: key.to_string(),
                node,
            });
        }
        PositionNode {
            position,
            children: finish_members(children),
        }
    }

    fn array(position: TextPosition, elements: Vec<PositionNode>) -> PositionNode {
        PositionNode {
            position,
            children: PositionChildren::Elements(elements),
        }
    }

    // {"inputs": [{"range": 1}], "a/b": {"c": 2}}, as positions only.
    fn sample_map() -> PositionMap {
        let first_input = object(at(3, 5), vec![("range", leaf(at(3, 6)))]);
        let inputs = array(at(2, 3), vec![first_input]);
        let escaped = object(at(4, 2), vec![("c", leaf(at(4, 10)))]);
        position_map_from_root(object(at(1, 1), vec![("inputs", inputs), ("a/b", escaped)]))
    }

    #[test]
    fn exact_lookup_and_parent_fallback() {
        let map = sample_map();
        assert_eq!(position_of(&map, ""), Some(at(1, 1)));
        assert_eq!(position_of(&map, "/inputs"), Some(at(2, 3)));
        assert_eq!(position_of(&map, "/inputs/0"), Some(at(3, 5)));
        assert_eq!(position_of(&map, "/inputs/0/range"), Some(at(3, 6)));
        assert_eq!(position_of(&map, "/inputs/0/missing"), None);
        assert_eq!(position_of(&map, "/inputs/1"), None);
        assert_eq!(find_position(&map, "/inputs/0/missing/deeper"), Some(at(3, 5)));
        assert_eq!(find_position(&map, "/inputs/7"), Some(at(2, 3)));
        assert_eq!(find_position(&map, "/missing/deep/field"), Some(at(1, 1)));
        // A leaf has no children, so a pointer past it falls back to the leaf.
        assert_eq!(find_position(&map, "/inputs/0/range/x"), Some(at(3, 6)));
        assert_eq!(position_count(&map), 6);
    }

    #[test]
    fn members_given_in_any_order_are_all_found() {
        // Keys in reverse order, as a document may write them; binary search needs them sorted.
        let keys = ["z", "y", "m", "b", "a", ""];
        let mut members = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            members.push((*key, leaf(at(2, index as u32 + 1))));
        }
        let map = position_map_from_root(object(at(1, 1), members));
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(position_of(&map, &format!("/{key}")), Some(at(2, index as u32 + 1)));
        }
        assert_eq!(position_of(&map, "/c"), None);
        assert_eq!(position_count(&map), keys.len() + 1);
    }

    #[test]
    fn empty_map_has_no_positions() {
        let map = PositionMap::default();
        assert_eq!(find_position(&map, "/a"), None);
        assert_eq!(find_position(&map, ""), None);
        assert_eq!(position_of(&map, ""), None);
        assert_eq!(position_count(&map), 0);
    }

    #[test]
    fn escaped_tokens_and_text_that_is_not_a_pointer() {
        let map = sample_map();
        assert_eq!(position_of(&map, "/a~1b/c"), Some(at(4, 10)));
        assert_eq!(find_position(&map, "/a~1b/missing"), Some(at(4, 2)));
        // An index with a leading zero, a bad escape, or no leading slash names nothing; the nearest parent is used.
        assert_eq!(position_of(&map, "/inputs/00"), None);
        assert_eq!(find_position(&map, "/inputs/00"), Some(at(2, 3)));
        assert_eq!(find_position(&map, "/a~2b/c"), Some(at(1, 1)));
        assert_eq!(position_of(&map, "inputs"), None);
        assert_eq!(find_position(&map, "inputs"), Some(at(1, 1)));
    }

    #[test]
    fn a_missing_field_points_at_the_object_that_should_hold_it() {
        let parsed = crate::json::reader::read_json(b"{\n  \"a\": {}\n}", &crate::limits::ImportLimits::default());
        let positions = parsed.map(|parsed| parsed.positions).unwrap_or_default();
        assert_eq!(diagnostic_path("f.json", &positions, "/a/b"), "f.json:2:3#/a/b");
        assert_eq!(diagnostic_path("f.json", &positions, ""), "f.json:1:1");
    }
}
