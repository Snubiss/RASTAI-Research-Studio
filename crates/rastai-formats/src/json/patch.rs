//! The JSON Patch subset undo needs (RFC 6902: add, remove, replace, move; flows/platform.md 3.4). An edit stores
//! two patches: the forward one (redo) and the inverse one (undo), both made by `diff_values`.
//!
//! `test` and `copy` are left out: undo never needs them, and a patch here is always one Rastai made itself.
//! `diff_values` never emits `move`; `apply_patch` accepts it so a patch written by hand or by a later version of
//! the differ still applies.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::FormatsError;
use crate::json::pointer::{join_pointer, parse_array_index, split_pointer};

/// The token that means "after the last element" when adding to an array (RFC 6902 section 4.1).
const APPEND_TOKEN: &str = "-";

// Why an operation failed; carried in `FormatsError::PatchFailed.reason`.
const NOT_A_POINTER: &str = "path is not a JSON pointer";
const MISSING_TARGET: &str = "no value at this path";
const MISSING_PARENT: &str = "the parent of this path doesn't exist";
const PARENT_NOT_CONTAINER: &str = "the parent of this path is not an object or array";
const BAD_ARRAY_INDEX: &str = "array index is not a number in range";
const REMOVE_ROOT: &str = "the whole document can't be removed";
const MOVE_INTO_ITSELF: &str = "a value can't be moved into itself";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    Add,
    Remove,
    Replace,
    Move,
}

/// One operation. `from` is used only by `move`; `value` only by `add` and `replace` (null otherwise).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonPatchOperation {
    pub op: PatchOp,
    pub path: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub value: Value,
}

/// Applies the operations in order to a copy of the document. The first operation that fails stops the patch and
/// the document is left untouched.
pub fn apply_patch(document: &Value, patch: &[JsonPatchOperation]) -> Result<Value, FormatsError> {
    let mut result = document.clone();
    for (operation_index, operation) in patch.iter().enumerate() {
        apply_operation(&mut result, operation).map_err(|reason| FormatsError::PatchFailed {
            operation_index,
            path: operation.path.clone(),
            reason,
        })?;
    }
    Ok(result)
}

/// The operations that turn `before` into `after`. Objects are compared key by key and arrays element by element
/// after trimming their common start and end, so one inserted or removed element is one operation, not a
/// rewrite of everything after it.
pub fn diff_values(before: &Value, after: &Value) -> Vec<JsonPatchOperation> {
    let mut operations = Vec::new();
    diff_at(before, after, "", &mut operations);
    operations
}

fn apply_operation(document: &mut Value, operation: &JsonPatchOperation) -> Result<(), &'static str> {
    match operation.op {
        PatchOp::Add => add_value(document, &operation.path, operation.value.clone()),
        PatchOp::Remove => remove_value(document, &operation.path).map(|_| ()),
        PatchOp::Replace => replace_value(document, &operation.path, operation.value.clone()),
        PatchOp::Move => move_value(document, &operation.from, &operation.path),
    }
}

fn add_value(document: &mut Value, path: &str, value: Value) -> Result<(), &'static str> {
    let tokens = split_pointer(path).ok_or(NOT_A_POINTER)?;
    let Some((last, parents)) = tokens.split_last() else {
        *document = value;
        return Ok(());
    };
    match find_value_mut(document, parents).ok_or(MISSING_PARENT)? {
        Value::Object(members) => {
            members.insert(last.clone(), value);
            Ok(())
        }
        Value::Array(elements) => {
            let index = if last == APPEND_TOKEN {
                elements.len()
            } else {
                parse_array_index(last).ok_or(BAD_ARRAY_INDEX)?
            };
            if index > elements.len() {
                return Err(BAD_ARRAY_INDEX);
            }
            elements.insert(index, value);
            Ok(())
        }
        _ => Err(PARENT_NOT_CONTAINER),
    }
}

fn remove_value(document: &mut Value, path: &str) -> Result<Value, &'static str> {
    let tokens = split_pointer(path).ok_or(NOT_A_POINTER)?;
    let Some((last, parents)) = tokens.split_last() else {
        return Err(REMOVE_ROOT);
    };
    match find_value_mut(document, parents).ok_or(MISSING_PARENT)? {
        Value::Object(members) => members.remove(last).ok_or(MISSING_TARGET),
        Value::Array(elements) => {
            let index = parse_array_index(last).ok_or(BAD_ARRAY_INDEX)?;
            if index >= elements.len() {
                return Err(BAD_ARRAY_INDEX);
            }
            Ok(elements.remove(index))
        }
        _ => Err(PARENT_NOT_CONTAINER),
    }
}

fn replace_value(document: &mut Value, path: &str, value: Value) -> Result<(), &'static str> {
    let tokens = split_pointer(path).ok_or(NOT_A_POINTER)?;
    let target = find_value_mut(document, &tokens).ok_or(MISSING_TARGET)?;
    *target = value;
    Ok(())
}

/// Remove at `from`, then add at `path`. Moving a value into its own descendant would orphan it, so it's refused.
fn move_value(document: &mut Value, from: &str, path: &str) -> Result<(), &'static str> {
    if from == path {
        return find_value_mut(document, &split_pointer(from).ok_or(NOT_A_POINTER)?)
            .map(|_| ())
            .ok_or(MISSING_TARGET);
    }
    if path.starts_with(from) && path[from.len()..].starts_with('/') {
        return Err(MOVE_INTO_ITSELF);
    }
    let value = remove_value(document, from)?;
    add_value(document, path, value)
}

/// Walks down the tokens; None when any step is missing.
fn find_value_mut<'a>(document: &'a mut Value, tokens: &[String]) -> Option<&'a mut Value> {
    let mut current = document;
    for token in tokens {
        current = match current {
            Value::Object(members) => members.get_mut(token)?,
            Value::Array(elements) => elements.get_mut(parse_array_index(token)?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn diff_at(before: &Value, after: &Value, pointer: &str, operations: &mut Vec<JsonPatchOperation>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(old_members), Value::Object(new_members)) => {
            diff_objects(old_members, new_members, pointer, operations)
        }
        (Value::Array(old_elements), Value::Array(new_elements)) => {
            diff_arrays(old_elements, new_elements, pointer, operations)
        }
        _ => operations.push(operation(PatchOp::Replace, pointer.to_string(), after.clone())),
    }
}

fn diff_objects(
    before: &serde_json::Map<String, Value>,
    after: &serde_json::Map<String, Value>,
    pointer: &str,
    operations: &mut Vec<JsonPatchOperation>,
) {
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        operations.push(operation(PatchOp::Remove, join_pointer(pointer, key), Value::Null));
    }
    for (key, old_value) in before {
        if let Some(new_value) = after.get(key) {
            diff_at(old_value, new_value, &join_pointer(pointer, key), operations);
        }
    }
    for (key, new_value) in after.iter().filter(|(key, _)| !before.contains_key(*key)) {
        operations.push(operation(PatchOp::Add, join_pointer(pointer, key), new_value.clone()));
    }
}

/// Trims the equal start and end, diffs the middle pairwise, then removes what `before` has left over or adds
/// what `after` has. Removals repeat at one index because each one shifts the rest left.
fn diff_arrays(before: &[Value], after: &[Value], pointer: &str, operations: &mut Vec<JsonPatchOperation>) {
    let shorter = before.len().min(after.len());
    let prefix = (0..shorter).take_while(|&index| before[index] == after[index]).count();
    let suffix = (0..shorter - prefix)
        .take_while(|&offset| before[before.len() - 1 - offset] == after[after.len() - 1 - offset])
        .count();
    let old_middle = &before[prefix..before.len() - suffix];
    let new_middle = &after[prefix..after.len() - suffix];
    let paired = old_middle.len().min(new_middle.len());
    for index in 0..paired {
        let element_pointer = join_pointer(pointer, &(prefix + index).to_string());
        diff_at(&old_middle[index], &new_middle[index], &element_pointer, operations);
    }
    let first_unpaired = join_pointer(pointer, &(prefix + paired).to_string());
    for _ in paired..old_middle.len() {
        operations.push(operation(PatchOp::Remove, first_unpaired.clone(), Value::Null));
    }
    for (index, new_value) in new_middle.iter().enumerate().skip(paired) {
        let element_pointer = join_pointer(pointer, &(prefix + index).to_string());
        operations.push(operation(PatchOp::Add, element_pointer, new_value.clone()));
    }
}

fn operation(op: PatchOp, path: String, value: Value) -> JsonPatchOperation {
    JsonPatchOperation {
        op,
        path,
        from: String::new(),
        value,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn op(op: PatchOp, path: &str, value: Value) -> JsonPatchOperation {
        operation(op, path.to_string(), value)
    }

    fn moved(from: &str, path: &str) -> JsonPatchOperation {
        JsonPatchOperation {
            op: PatchOp::Move,
            path: path.to_string(),
            from: from.to_string(),
            value: Value::Null,
        }
    }

    fn assert_applies(document: Value, operation: JsonPatchOperation, expected: Value) {
        let patched = apply_patch(&document, std::slice::from_ref(&operation)).unwrap();
        assert_eq!(patched, expected, "{operation:?}");
    }

    // RFC 6902 appendix A examples for add, remove and replace.
    #[test]
    fn add_remove_and_replace_follow_the_rfc_examples() {
        let foo_bar = json!({"foo": "bar"});
        let baz_qux = json!({"baz": "qux", "foo": "bar"});
        assert_applies(foo_bar.clone(), op(PatchOp::Add, "/baz", json!("qux")), baz_qux.clone());
        let list = json!({"foo": ["bar", "qux", "baz"]});
        assert_applies(
            json!({"foo": ["bar", "baz"]}),
            op(PatchOp::Add, "/foo/1", json!("qux")),
            list.clone(),
        );
        assert_applies(
            baz_qux.clone(),
            op(PatchOp::Remove, "/baz", Value::Null),
            foo_bar.clone(),
        );
        assert_applies(
            list,
            op(PatchOp::Remove, "/foo/1", Value::Null),
            json!({"foo": ["bar", "baz"]}),
        );
        let replaced = json!({"baz": "boo", "foo": "bar"});
        assert_applies(baz_qux, op(PatchOp::Replace, "/baz", json!("boo")), replaced);
        let child = json!({"grandchild": {}});
        let with_child = json!({"foo": "bar", "child": {"grandchild": {}}});
        assert_applies(foo_bar, op(PatchOp::Add, "/child", child), with_child);
        let appended = json!({"foo": ["bar", ["abc", "def"]]});
        assert_applies(
            json!({"foo": ["bar"]}),
            op(PatchOp::Add, "/foo/-", json!(["abc", "def"])),
            appended,
        );
        assert_applies(
            json!({"foo": null}),
            op(PatchOp::Replace, "/foo", json!(1)),
            json!({"foo": 1}),
        );
    }

    // RFC 6902 appendix A examples for move, and operations on the whole document.
    #[test]
    fn move_and_whole_document_operations() {
        let before = json!({"foo": {"bar": "baz", "waldo": "fred"}, "qux": {"corge": "grault"}});
        let after = json!({"foo": {"bar": "baz"}, "qux": {"corge": "grault", "thud": "fred"}});
        assert_applies(before, moved("/foo/waldo", "/qux/thud"), after);
        let before = json!({"foo": ["all", "grass", "cows", "eat"]});
        let after = json!({"foo": ["all", "cows", "eat", "grass"]});
        assert_applies(before, moved("/foo/1", "/foo/3"), after);
        assert_applies(json!({"a": {"b": 1}}), moved("/a", "/a"), json!({"a": {"b": 1}}));
        assert_applies(json!({"a": 1}), op(PatchOp::Replace, "", json!([1])), json!([1]));
        assert_applies(json!({"a": 1}), op(PatchOp::Add, "", json!("whole")), json!("whole"));
    }

    #[test]
    fn failures_name_the_operation_and_leave_the_document_alone() {
        let document = json!({"foo": "bar", "list": [1, 2]});
        let failing = [
            (op(PatchOp::Add, "/baz/bat", json!("qux")), MISSING_PARENT),
            (op(PatchOp::Add, "/list/3", json!(0)), BAD_ARRAY_INDEX),
            (op(PatchOp::Add, "/list/01", json!(0)), BAD_ARRAY_INDEX),
            (op(PatchOp::Add, "/foo/x", json!(0)), PARENT_NOT_CONTAINER),
            (op(PatchOp::Remove, "/missing", Value::Null), MISSING_TARGET),
            (op(PatchOp::Remove, "/list/2", Value::Null), BAD_ARRAY_INDEX),
            (op(PatchOp::Remove, "/list/-", Value::Null), BAD_ARRAY_INDEX),
            (op(PatchOp::Remove, "", Value::Null), REMOVE_ROOT),
            (op(PatchOp::Replace, "/missing", json!(1)), MISSING_TARGET),
            (op(PatchOp::Replace, "no-slash", json!(1)), NOT_A_POINTER),
            (moved("/list", "/list/0"), MOVE_INTO_ITSELF),
            (moved("/missing", "/x"), MISSING_TARGET),
        ];
        for (failing_operation, reason) in failing {
            let patch = [op(PatchOp::Add, "/first", json!(1)), failing_operation.clone()];
            let error = apply_patch(&document, &patch).unwrap_err();
            assert_eq!(
                error,
                FormatsError::PatchFailed {
                    operation_index: 1,
                    path: failing_operation.path.clone(),
                    reason
                }
            );
            assert_eq!(error.code(), "RRS-E2001");
        }
        // "/list2" starts with "/list" but isn't inside it, so the move is fine.
        let renamed = apply_patch(&document, &[moved("/list", "/list2")]).unwrap();
        assert_eq!(renamed, json!({"foo": "bar", "list2": [1, 2]}));
    }

    #[test]
    fn diff_gives_small_patches_that_apply() {
        let before = json!({"name": "a", "gone": 1, "nodes": [{"id": 1}, {"id": 2}, {"id": 3}], "k": {"x": [1]}});
        let after = json!({"name": "b", "new": true, "nodes": [{"id": 0}, {"id": 1}, {"id": 3}], "k": {"x": [1, 2]}});
        let patch = diff_values(&before, &after);
        assert_eq!(apply_patch(&before, &patch).unwrap(), after);
        // An inserted element is one add, not a rewrite of every element after it.
        let inserted = diff_values(&json!([1, 2, 3, 4]), &json!([1, 9, 2, 3, 4]));
        assert_eq!(inserted, vec![op(PatchOp::Add, "/1", json!(9))]);
        let removed = diff_values(&json!([1, 2, 3, 4]), &json!([1, 4]));
        assert_eq!(
            removed,
            vec![
                op(PatchOp::Remove, "/1", Value::Null),
                op(PatchOp::Remove, "/1", Value::Null)
            ]
        );
        assert!(diff_values(&after, &after).is_empty());
        assert_eq!(
            diff_values(&json!(1), &json!("1")),
            vec![op(PatchOp::Replace, "", json!("1"))]
        );
        let escaped = diff_values(&json!({"a/b": 1}), &json!({"a/b": 2, "m~n": 3}));
        assert_eq!(
            escaped,
            vec![
                op(PatchOp::Replace, "/a~1b", json!(2)),
                op(PatchOp::Add, "/m~0n", json!(3))
            ]
        );
    }

    #[test]
    fn operations_serialize_as_json_patch() {
        let operation = op(PatchOp::Replace, "/a", json!(1));
        let text = serde_json::to_string(&operation).unwrap();
        assert_eq!(text, r#"{"op":"replace","path":"/a","from":"","value":1}"#);
        let read_back: JsonPatchOperation = serde_json::from_str(r#"{"op":"remove","path":"/a"}"#).unwrap();
        assert_eq!(read_back, op(PatchOp::Remove, "/a", Value::Null));
    }

    // Seeded random documents and random edits of them (the in-house generator of layout 4.11, no
    // property-testing crate): the forward patch turns before into after, and the inverse patch turns it back.
    #[test]
    fn forward_and_inverse_patches_round_trip_on_random_documents() {
        let mut seed: u64 = 0xD1FF_0001;
        for _ in 0..2_000 {
            let before = random_value(&mut seed, 3);
            let mut after = before.clone();
            for _ in 0..1 + next_random(&mut seed) % 4 {
                random_edit(&mut after, &mut seed);
            }
            let forward = diff_values(&before, &after);
            let inverse = diff_values(&after, &before);
            assert_eq!(apply_patch(&before, &forward).unwrap(), after, "forward {forward:?}");
            assert_eq!(apply_patch(&after, &inverse).unwrap(), before, "inverse {inverse:?}");
        }
    }

    // Keys include the two characters a pointer must escape.
    const RANDOM_KEYS: [&str; 6] = ["a", "b", "c/d", "e~f", "", "g"];

    fn random_value(seed: &mut u64, depth: u32) -> Value {
        let kind = if depth == 0 {
            next_random(seed) % 4
        } else {
            next_random(seed) % 6
        };
        match kind {
            0 => Value::Null,
            1 => json!(next_random(seed).is_multiple_of(2)),
            2 => json!(next_random(seed) % 10),
            3 => json!(RANDOM_KEYS[(next_random(seed) % 6) as usize]),
            4 => Value::Array(
                (0..next_random(seed) % 5)
                    .map(|_| random_value(seed, depth - 1))
                    .collect(),
            ),
            _ => {
                let mut members = serde_json::Map::new();
                for _ in 0..next_random(seed) % 5 {
                    let key = RANDOM_KEYS[(next_random(seed) % 6) as usize].to_string();
                    members.insert(key, random_value(seed, depth - 1));
                }
                Value::Object(members)
            }
        }
    }

    /// One random change somewhere in the value: replace, insert or remove.
    fn random_edit(value: &mut Value, seed: &mut u64) {
        let roll = next_random(seed) % 4;
        match value {
            Value::Array(elements) if !elements.is_empty() && roll < 2 => {
                let index = (next_random(seed) as usize) % elements.len();
                if roll == 0 {
                    random_edit(&mut elements[index], seed);
                } else {
                    elements.remove(index);
                }
            }
            Value::Array(elements) if roll == 2 => {
                let index = (next_random(seed) as usize) % (elements.len() + 1);
                elements.insert(index, random_value(seed, 1));
            }
            Value::Object(members) if !members.is_empty() && roll < 2 => {
                let key = members
                    .keys()
                    .nth((next_random(seed) as usize) % members.len())
                    .unwrap()
                    .clone();
                if roll == 0 {
                    random_edit(members.get_mut(&key).unwrap(), seed);
                } else {
                    members.remove(&key);
                }
            }
            Value::Object(members) if roll == 2 => {
                let key = RANDOM_KEYS[(next_random(seed) % 6) as usize].to_string();
                members.insert(key, random_value(seed, 1));
            }
            _ => *value = random_value(seed, 2),
        }
    }

    fn next_random(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *seed >> 33
    }
}
