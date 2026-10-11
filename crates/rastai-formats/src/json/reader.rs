//! The strict JSON reader every document goes through (layer 1, ADR-0032). In-house, because serde_json accepts
//! duplicate keys (the last one silently wins) and doesn't keep positions.
//!
//! What it refuses, so Rust, the browser and Python agree on what "valid JSON" means (spike 07):
//! - bytes that aren't UTF-8 (`RRS-E0002`), a byte order mark, comments, single quotes, trailing commas, leading
//!   zeros, raw control characters in strings, lone UTF-16 surrogates, and anything after the value (`RRS-E0001`);
//! - an object with the same key twice (`RRS-E0003`);
//! - `NaN`, `Infinity`, `-Infinity`, and numbers too big for a 64-bit float (`RRS-E0004`);
//! - input over `max_file_bytes` (`RRS-E5003`), nested deeper than `max_json_depth` (`RRS-E5004`), or with more
//!   than `max_json_values` values (`RRS-E5006`).
//!
//! Numbers become serde_json numbers the way serde_json makes them: an integer that fits u64 or i64 stays an
//! integer, everything else is an f64. Every value's position goes into the `PositionMap`, a tree shaped like the
//! value, so the reader never builds a pointer string per value (positions.rs says why).
//!
//! The reader is recursive; the depth limit is checked before each nested array or object, so recursion never
//! goes deeper than `max_json_depth`. The value count is checked before each value, so a document of tiny values
//! (`[0,0,0,...]`) is refused before its tree outgrows the limits.rs budget.

use serde_json::{Map, Number, Value};

use crate::error::FormatsError;
use crate::json::pointer::{ROOT_POINTER, join_pointer};
use crate::json::positions::{
    MemberPosition, PositionChildren, PositionMap, PositionNode, TextPosition, finish_elements, finish_members,
    position_map_from_root,
};
use crate::limits::ImportLimits;

// What the reader wanted where it stopped; carried in `FormatsError::JsonSyntax.expected`.
const EXPECTED_VALUE: &str = "a value";
const EXPECTED_KEY: &str = "a string key";
const EXPECTED_COLON: &str = "':'";
const EXPECTED_MEMBER_END: &str = "',' or '}'";
const EXPECTED_ELEMENT_END: &str = "',' or ']'";
const EXPECTED_END: &str = "end of input";
const EXPECTED_DIGIT: &str = "a digit";
const EXPECTED_STRING_END: &str = "'\"' to close the string";
const EXPECTED_ESCAPE: &str = "an escape: \\\" \\\\ \\/ \\b \\f \\n \\r \\t or \\u";
const EXPECTED_HEX: &str = "four hex digits";
const EXPECTED_LOW_SURROGATE: &str = "a \\u escape for the second half of a surrogate pair";
const EXPECTED_ESCAPED_CONTROL: &str = "a control character written as an escape";

// The words other JSON writers use for numbers JSON can't hold (Python's json.dumps writes all three).
const NON_FINITE_WORDS: [&str; 3] = ["NaN", "Infinity", "-Infinity"];

// UTF-16 surrogates, for `\uD83D\uDE00`-style escapes of characters outside the Basic Multilingual Plane.
const HIGH_SURROGATE_FIRST: u32 = 0xD800;
const HIGH_SURROGATE_LAST: u32 = 0xDBFF;
const LOW_SURROGATE_FIRST: u32 = 0xDC00;
const LOW_SURROGATE_LAST: u32 = 0xDFFF;
const SURROGATE_PAIR_BASE: u32 = 0x10000;
const HEX_DIGITS_PER_ESCAPE: usize = 4;

/// A document the reader accepted: its value and where each part of it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedJson {
    pub value: Value,
    pub positions: PositionMap,
}

/// The reader's state: the text, how far it got, the line and column of that point, and the two counts that
/// limits.rs caps.
struct Cursor<'a> {
    text: &'a str,
    offset: usize,
    line: u32,
    column: u32,
    depth: u32,
    max_depth: u32,
    value_count: u64,
    max_values: u64,
}

/// A value the reader finished, with the positions of everything inside it. Its own position belongs to the
/// caller, because a member's position is its key's, not its value's.
struct ReadValue {
    value: Value,
    children: PositionChildren,
}

/// Reads one JSON document from bytes. The first problem stops the read; JSON past a syntax error can't be
/// trusted to mean anything.
pub fn read_json(bytes: &[u8], limits: &ImportLimits) -> Result<ParsedJson, FormatsError> {
    check_size(bytes, limits)?;
    let text = check_utf8(bytes)?;
    let mut cursor = Cursor {
        text,
        offset: 0,
        line: 1,
        column: 1,
        depth: 0,
        max_depth: limits.max_json_depth,
        value_count: 0,
        max_values: limits.max_json_values,
    };
    skip_whitespace(&mut cursor);
    let root_position = position(&cursor);
    let root = read_value(&mut cursor)?;
    skip_whitespace(&mut cursor);
    if cursor.offset < cursor.text.len() {
        return Err(syntax_error(&cursor, EXPECTED_END));
    }
    let root_node = PositionNode {
        position: root_position,
        children: root.children,
    };
    Ok(ParsedJson {
        value: root.value,
        positions: position_map_from_root(root_node),
    })
}

fn check_size(bytes: &[u8], limits: &ImportLimits) -> Result<(), FormatsError> {
    let size_bytes = bytes.len() as u64;
    if size_bytes > limits.max_file_bytes {
        return Err(FormatsError::FileTooLarge {
            size_bytes,
            limit_bytes: limits.max_file_bytes,
        });
    }
    Ok(())
}

fn check_utf8(bytes: &[u8]) -> Result<&str, FormatsError> {
    std::str::from_utf8(bytes).map_err(|error| FormatsError::NotUtf8 {
        position: position_at_offset(bytes, error.valid_up_to()),
    })
}

/// The line and column of a byte offset, counted from the start. Used once, for the UTF-8 error; the reader
/// itself keeps its position as it goes.
fn position_at_offset(bytes: &[u8], offset: usize) -> TextPosition {
    let mut place = TextPosition { line: 1, column: 1 };
    for &byte in &bytes[..offset.min(bytes.len())] {
        step_position(&mut place, byte);
    }
    place
}

/// Moves a position past one byte. A newline starts a new line; a UTF-8 continuation byte belongs to the
/// character before it, so only the first byte of each character moves the column.
fn step_position(place: &mut TextPosition, byte: u8) {
    if byte == b'\n' {
        place.line = place.line.saturating_add(1);
        place.column = 1;
    } else if !is_continuation_byte(byte) {
        place.column = place.column.saturating_add(1);
    }
}

fn is_continuation_byte(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn read_value(cursor: &mut Cursor) -> Result<ReadValue, FormatsError> {
    count_value(cursor)?;
    if NON_FINITE_WORDS.iter().any(|word| starts_with(cursor, word)) {
        return Err(FormatsError::NotANumber {
            position: position(cursor),
        });
    }
    let value = match peek(cursor) {
        Some(b'{') => return read_object(cursor),
        Some(b'[') => return read_array(cursor),
        Some(b'"') => Value::String(read_string(cursor)?),
        Some(b't') => read_literal(cursor, "true", Value::Bool(true))?,
        Some(b'f') => read_literal(cursor, "false", Value::Bool(false))?,
        Some(b'n') => read_literal(cursor, "null", Value::Null)?,
        Some(b'-' | b'0'..=b'9') => read_number(cursor)?,
        _ => return Err(syntax_error(cursor, EXPECTED_VALUE)),
    };
    Ok(ReadValue {
        value,
        children: PositionChildren::Leaf,
    })
}

/// Counts one more value, refusing the one past `max_json_values`. Every value costs memory in the tree and the
/// position map whatever its size in the text (`0,` is two bytes), so the byte limit alone doesn't bound memory.
fn count_value(cursor: &mut Cursor) -> Result<(), FormatsError> {
    if cursor.value_count >= cursor.max_values {
        return Err(FormatsError::TooManyValues {
            position: position(cursor),
            limit_count: cursor.max_values,
        });
    }
    cursor.value_count += 1;
    Ok(())
}

fn read_object(cursor: &mut Cursor) -> Result<ReadValue, FormatsError> {
    enter_container(cursor)?;
    let mut members = Map::new();
    // Room for one member: a list that grows from empty reserves four, and most objects are small. A hostile
    // document of one-member objects spends about 14% less memory this way; finish_members trims a bigger list.
    let mut member_positions = Vec::with_capacity(1);
    skip_whitespace(cursor);
    if peek(cursor) != Some(b'}') {
        loop {
            skip_whitespace(cursor);
            read_member(cursor, &mut members, &mut member_positions)?;
            skip_whitespace(cursor);
            match peek(cursor) {
                Some(b',') => advance(cursor, 1),
                Some(b'}') => break,
                _ => return Err(syntax_error(cursor, EXPECTED_MEMBER_END)),
            }
        }
    }
    leave_container(cursor);
    Ok(ReadValue {
        value: Value::Object(members),
        children: finish_members(member_positions),
    })
}

/// One `"key": value` pair. The key's position is the member's position; a repeated key is refused there.
fn read_member(
    cursor: &mut Cursor,
    members: &mut Map<String, Value>,
    member_positions: &mut Vec<MemberPosition>,
) -> Result<(), FormatsError> {
    if peek(cursor) != Some(b'"') {
        return Err(syntax_error(cursor, EXPECTED_KEY));
    }
    let key_position = position(cursor);
    let key = read_string(cursor)?;
    if members.contains_key(&key) {
        return Err(FormatsError::DuplicateKey {
            pointer: join_pointer(ROOT_POINTER, &key),
            position: key_position,
        });
    }
    skip_whitespace(cursor);
    if peek(cursor) != Some(b':') {
        return Err(syntax_error(cursor, EXPECTED_COLON));
    }
    advance(cursor, 1);
    skip_whitespace(cursor);
    let member = read_value(cursor).map_err(|error| add_parent_token(error, &key))?;
    member_positions.push(MemberPosition {
        key: key.clone(),
        node: PositionNode {
            position: key_position,
            children: member.children,
        },
    });
    members.insert(key, member.value);
    Ok(())
}

fn read_array(cursor: &mut Cursor) -> Result<ReadValue, FormatsError> {
    enter_container(cursor)?;
    let mut elements = Vec::new();
    let mut element_positions = Vec::new();
    skip_whitespace(cursor);
    if peek(cursor) != Some(b']') {
        loop {
            skip_whitespace(cursor);
            let element_position = position(cursor);
            let index = elements.len();
            let element = read_value(cursor).map_err(|error| add_parent_token(error, &index.to_string()))?;
            element_positions.push(PositionNode {
                position: element_position,
                children: element.children,
            });
            elements.push(element.value);
            skip_whitespace(cursor);
            match peek(cursor) {
                Some(b',') => advance(cursor, 1),
                Some(b']') => break,
                _ => return Err(syntax_error(cursor, EXPECTED_ELEMENT_END)),
            }
        }
    }
    leave_container(cursor);
    Ok(ReadValue {
        value: Value::Array(elements),
        children: finish_elements(element_positions),
    })
}

/// A duplicate key's pointer is built on the way back out of the recursion: the object that found it names only
/// the key, and each enclosing member or element puts its own token in front. Only this error carries a pointer,
/// and building it here costs nothing for documents that have no duplicate.
fn add_parent_token(error: FormatsError, token: &str) -> FormatsError {
    match error {
        FormatsError::DuplicateKey { pointer, position } => {
            let mut full_pointer = join_pointer(ROOT_POINTER, token);
            full_pointer.push_str(&pointer);
            FormatsError::DuplicateKey {
                pointer: full_pointer,
                position,
            }
        }
        other => other,
    }
}

/// Steps over an opening `{` or `[`, after checking the nesting limit at that bracket.
fn enter_container(cursor: &mut Cursor) -> Result<(), FormatsError> {
    if cursor.depth >= cursor.max_depth {
        return Err(FormatsError::TooDeep {
            position: position(cursor),
            limit_depth: cursor.max_depth,
        });
    }
    cursor.depth += 1;
    advance(cursor, 1);
    Ok(())
}

/// Steps over the closing `}` or `]` that `read_object` or `read_array` stopped at.
fn leave_container(cursor: &mut Cursor) {
    advance(cursor, 1);
    cursor.depth -= 1;
}

/// A string, from its opening quote to its closing one. Plain runs are copied whole; escapes one at a time.
fn read_string(cursor: &mut Cursor) -> Result<String, FormatsError> {
    advance(cursor, 1);
    let mut text = String::new();
    loop {
        let run_end = plain_run_end(cursor);
        if run_end > cursor.offset {
            let run = slice(cursor, cursor.offset, run_end)?;
            text.push_str(run);
            advance(cursor, run_end - cursor.offset);
        }
        match peek(cursor) {
            Some(b'"') => {
                advance(cursor, 1);
                return Ok(text);
            }
            Some(b'\\') => read_escape(cursor, &mut text)?,
            Some(_) => return Err(syntax_error(cursor, EXPECTED_ESCAPED_CONTROL)),
            None => return Err(syntax_error(cursor, EXPECTED_STRING_END)),
        }
    }
}

/// Where the run of characters that need no special handling ends: at a quote, a backslash or a control
/// character. Those are all ASCII, so the run ends on a character boundary.
fn plain_run_end(cursor: &Cursor) -> usize {
    let bytes = cursor.text.as_bytes();
    let mut end = cursor.offset;
    while end < bytes.len() && bytes[end] != b'"' && bytes[end] != b'\\' && bytes[end] >= 0x20 {
        end += 1;
    }
    end
}

/// One backslash escape. `\uXXXX` may be the first half of a surrogate pair, which must be followed by the
/// second half: Rust strings (and so serde_json values) can't hold a lone surrogate.
fn read_escape(cursor: &mut Cursor, text: &mut String) -> Result<(), FormatsError> {
    let escape_position = position(cursor);
    advance(cursor, 1);
    let simple = match peek(cursor) {
        Some(b'"') => Some('"'),
        Some(b'\\') => Some('\\'),
        Some(b'/') => Some('/'),
        Some(b'b') => Some('\u{0008}'),
        Some(b'f') => Some('\u{000C}'),
        Some(b'n') => Some('\n'),
        Some(b'r') => Some('\r'),
        Some(b't') => Some('\t'),
        Some(b'u') => None,
        _ => return Err(syntax_error(cursor, EXPECTED_ESCAPE)),
    };
    advance(cursor, 1);
    let character = match simple {
        Some(character) => character,
        None => read_unicode_escape(cursor, escape_position)?,
    };
    text.push(character);
    Ok(())
}

/// The four hex digits after `\u`, plus the second `\uXXXX` when the first is a high surrogate.
fn read_unicode_escape(cursor: &mut Cursor, escape_position: TextPosition) -> Result<char, FormatsError> {
    let first = read_hex4(cursor)?;
    let code_point = if (HIGH_SURROGATE_FIRST..=HIGH_SURROGATE_LAST).contains(&first) {
        if !starts_with(cursor, "\\u") {
            return Err(syntax_error(cursor, EXPECTED_LOW_SURROGATE));
        }
        advance(cursor, 2);
        let second = read_hex4(cursor)?;
        if !(LOW_SURROGATE_FIRST..=LOW_SURROGATE_LAST).contains(&second) {
            return Err(syntax_error(cursor, EXPECTED_LOW_SURROGATE));
        }
        // Each half carries 10 bits of the code point above U+FFFF.
        SURROGATE_PAIR_BASE + ((first - HIGH_SURROGATE_FIRST) << 10) + (second - LOW_SURROGATE_FIRST)
    } else {
        first
    };
    // None only for a lone low surrogate: every other value here is a valid scalar value.
    char::from_u32(code_point).ok_or(FormatsError::JsonSyntax {
        position: escape_position,
        expected: EXPECTED_LOW_SURROGATE,
    })
}

fn read_hex4(cursor: &mut Cursor) -> Result<u32, FormatsError> {
    let end = cursor.offset + HEX_DIGITS_PER_ESCAPE;
    let digits = slice(cursor, cursor.offset, end).map_err(|_| syntax_error(cursor, EXPECTED_HEX))?;
    // from_str_radix alone would also take a leading `+`.
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(syntax_error(cursor, EXPECTED_HEX));
    }
    let value = u32::from_str_radix(digits, 16).map_err(|_| syntax_error(cursor, EXPECTED_HEX))?;
    advance(cursor, HEX_DIGITS_PER_ESCAPE);
    Ok(value)
}

fn read_literal(cursor: &mut Cursor, word: &str, value: Value) -> Result<Value, FormatsError> {
    if !starts_with(cursor, word) {
        return Err(syntax_error(cursor, EXPECTED_VALUE));
    }
    advance(cursor, word.len());
    Ok(value)
}

/// A number in RFC 8259's grammar: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`. A leading zero
/// followed by more digits stops after the zero, and the digits that follow are then refused as trailing text.
fn read_number(cursor: &mut Cursor) -> Result<Value, FormatsError> {
    let start = cursor.offset;
    let start_position = position(cursor);
    if peek(cursor) == Some(b'-') {
        advance(cursor, 1);
    }
    match peek(cursor) {
        Some(b'0') => advance(cursor, 1),
        Some(b'1'..=b'9') => skip_digits(cursor),
        _ => return Err(syntax_error(cursor, EXPECTED_DIGIT)),
    }
    let mut is_integer = true;
    if peek(cursor) == Some(b'.') {
        advance(cursor, 1);
        require_digits(cursor)?;
        is_integer = false;
    }
    if matches!(peek(cursor), Some(b'e' | b'E')) {
        advance(cursor, 1);
        if matches!(peek(cursor), Some(b'+' | b'-')) {
            advance(cursor, 1);
        }
        require_digits(cursor)?;
        is_integer = false;
    }
    let text = slice(cursor, start, cursor.offset)?;
    number_value(text, is_integer, start_position)
}

/// The serde_json number for a number's text: u64, then i64, then f64, as serde_json itself chooses. `-0` is the
/// float -0.0, as in serde_json, so both readers give equal values.
fn number_value(text: &str, is_integer: bool, start_position: TextPosition) -> Result<Value, FormatsError> {
    if is_integer && text != "-0" {
        if let Ok(unsigned) = text.parse::<u64>() {
            return Ok(Value::Number(Number::from(unsigned)));
        }
        if let Ok(signed) = text.parse::<i64>() {
            return Ok(Value::Number(Number::from(signed)));
        }
    }
    let not_a_number = FormatsError::NotANumber {
        position: start_position,
    };
    // Rust's float parsing is correctly rounded and turns a too-large number into infinity, which JSON can't hold.
    let float: f64 = text.parse().map_err(|_| not_a_number.clone())?;
    Number::from_f64(float).map(Value::Number).ok_or(not_a_number)
}

fn require_digits(cursor: &mut Cursor) -> Result<(), FormatsError> {
    if !matches!(peek(cursor), Some(b'0'..=b'9')) {
        return Err(syntax_error(cursor, EXPECTED_DIGIT));
    }
    skip_digits(cursor);
    Ok(())
}

fn skip_digits(cursor: &mut Cursor) {
    while matches!(peek(cursor), Some(b'0'..=b'9')) {
        advance(cursor, 1);
    }
}

/// JSON whitespace only: space, tab, newline, carriage return. A byte order mark is not whitespace.
fn skip_whitespace(cursor: &mut Cursor) {
    while matches!(peek(cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        advance(cursor, 1);
    }
}

fn peek(cursor: &Cursor) -> Option<u8> {
    cursor.text.as_bytes().get(cursor.offset).copied()
}

fn starts_with(cursor: &Cursor, word: &str) -> bool {
    cursor.text.as_bytes()[cursor.offset..].starts_with(word.as_bytes())
}

/// Moves forward `count` bytes, keeping the line and column up to date.
fn advance(cursor: &mut Cursor, count: usize) {
    let end = (cursor.offset + count).min(cursor.text.len());
    let mut place = position(cursor);
    for &byte in &cursor.text.as_bytes()[cursor.offset..end] {
        step_position(&mut place, byte);
    }
    cursor.line = place.line;
    cursor.column = place.column;
    cursor.offset = end;
}

/// A piece of the text. The reader only cuts at ASCII bytes, so this can't fail on valid input; a failure is
/// reported as a syntax error rather than a panic.
fn slice<'a>(cursor: &Cursor<'a>, start: usize, end: usize) -> Result<&'a str, FormatsError> {
    cursor
        .text
        .get(start..end)
        .ok_or_else(|| syntax_error(cursor, EXPECTED_VALUE))
}

fn position(cursor: &Cursor) -> TextPosition {
    TextPosition {
        line: cursor.line,
        column: cursor.column,
    }
}

fn syntax_error(cursor: &Cursor, expected: &'static str) -> FormatsError {
    FormatsError::JsonSyntax {
        position: position(cursor),
        expected,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::json::positions::{position_count, position_of};
    use crate::test_random::next_random;
    use serde_json::json;

    fn read(text: &str) -> Result<ParsedJson, FormatsError> {
        read_json(text.as_bytes(), &ImportLimits::default())
    }

    fn at(line: u32, column: u32) -> TextPosition {
        TextPosition { line, column }
    }

    #[test]
    fn positions_of_members_elements_and_root() {
        let text = "{\n  \"name\": \"textcnn\",\n  \"inputs\": [\n    1,\n    {\"range\": [0, 5]}\n  ]\n}\n";
        let parsed = read(text).unwrap();
        let positions = &parsed.positions;
        assert_eq!(position_of(positions, ""), Some(at(1, 1)));
        assert_eq!(position_of(positions, "/name"), Some(at(2, 3)));
        assert_eq!(position_of(positions, "/inputs"), Some(at(3, 3)));
        assert_eq!(position_of(positions, "/inputs/0"), Some(at(4, 5)));
        assert_eq!(position_of(positions, "/inputs/1"), Some(at(5, 5)));
        assert_eq!(position_of(positions, "/inputs/1/range"), Some(at(5, 6)));
        assert_eq!(position_of(positions, "/inputs/1/range/1"), Some(at(5, 19)));
        // One position per value: root, name, inputs, two elements, range and its two numbers.
        assert_eq!(position_count(positions), 8);
    }

    #[test]
    fn columns_count_characters_and_crlf_is_one_line_break() {
        // "é" is two bytes and "😀" four; each is one column.
        let parsed = read("{\"é😀\": 1, \"b\": 2}").unwrap();
        assert_eq!(position_of(&parsed.positions, "/b"), Some(at(1, 11)));
        let parsed = read("{\r\n\"a\": 1,\r\n  \"b\": 2}").unwrap();
        assert_eq!(position_of(&parsed.positions, "/b"), Some(at(3, 3)));
    }

    #[test]
    fn keys_are_escaped_in_pointers() {
        let parsed = read(r#"{"a/b": {"m~n": 1}}"#).unwrap();
        assert_eq!(position_of(&parsed.positions, "/a~1b/m~0n"), Some(at(1, 10)));
    }

    #[test]
    fn duplicate_key_is_e0003_at_the_second_key() {
        let error = read("{\n  \"a\": 1,\n  \"a\": 2\n}").unwrap_err();
        assert_eq!(error.code(), "RRS-E0003");
        assert_eq!(
            error,
            FormatsError::DuplicateKey {
                pointer: "/a".to_string(),
                position: at(3, 3)
            }
        );
        let nested = read(r#"{"outer": [{"x": 1, "y": 2, "x": 3}]}"#).unwrap_err();
        assert_eq!(
            nested,
            FormatsError::DuplicateKey {
                pointer: "/outer/0/x".to_string(),
                position: at(1, 29)
            }
        );
        let escaped = read(r#"{"a/b": [0, {"m~n": 1, "m~n": 2}]}"#).unwrap_err();
        assert_eq!(
            escaped,
            FormatsError::DuplicateKey {
                pointer: "/a~1b/1/m~0n".to_string(),
                position: at(1, 24)
            }
        );
        // The same key in two different objects is fine.
        assert!(read(r#"{"a": {"k": 1}, "b": {"k": 2}}"#).is_ok());
    }

    #[test]
    fn nan_and_infinity_are_e0004() {
        let cases = [
            ("NaN", at(1, 1)),
            ("Infinity", at(1, 1)),
            ("-Infinity", at(1, 1)),
            ("{\"loss\": NaN}", at(1, 10)),
            ("[1, -Infinity]", at(1, 5)),
            ("1e400", at(1, 1)),
            ("[-1e999]", at(1, 2)),
        ];
        for (text, expected_position) in cases {
            let error = read(text).unwrap_err();
            assert_eq!(error.code(), "RRS-E0004", "{text}");
            assert_eq!(
                error,
                FormatsError::NotANumber {
                    position: expected_position
                },
                "{text}"
            );
        }
    }

    #[test]
    fn trailing_garbage_is_a_syntax_error_where_it_starts() {
        let cases = [
            ("{} x", at(1, 4)),
            ("{}{}", at(1, 3)),
            ("1 2", at(1, 3)),
            ("01", at(1, 2)),
            ("\"a\"\n\n,", at(3, 1)),
            ("[1]\u{0000}", at(1, 4)),
        ];
        for (text, expected_position) in cases {
            let error = read(text).unwrap_err();
            assert_eq!(
                error,
                FormatsError::JsonSyntax {
                    position: expected_position,
                    expected: EXPECTED_END
                },
                "{text:?}"
            );
            assert_eq!(error.code(), "RRS-E0001");
        }
    }

    #[test]
    fn strict_syntax_refusals() {
        let refused = [
            "",
            "   ",
            "\u{FEFF}{}",
            "{\"a\": 1,}",
            "[1, 2,]",
            "[,1]",
            "{'a': 1}",
            "{\"a\" 1}",
            "{a: 1}",
            "// comment\n{}",
            "{\"a\": 1 /* c */}",
            "\"unterminated",
            "\"tab\there\"",
            "\"new\nline\"",
            "\"\\x41\"",
            "\"\\u12G4\"",
            "\"\\u12\"",
            "\"\\ud83d\"",
            "\"\\ud83d\\u0041\"",
            "\"\\ude00\"",
            "-",
            "-a",
            "1.",
            ".5",
            "1e",
            "1e+",
            "+1",
            "tru",
            "nul",
            "True",
            "[1 2]",
            "{\"a\": 1 \"b\": 2}",
        ];
        for text in refused {
            let error = read(text).unwrap_err();
            assert_eq!(error.code(), "RRS-E0001", "{text:?} gave {error:?}");
        }
    }

    #[test]
    fn invalid_utf8_is_e0002_at_the_bad_byte() {
        let bytes = b"{\n  \"name\": \"ab\xFFcd\"\n}";
        let error = read_json(bytes, &ImportLimits::default()).unwrap_err();
        assert_eq!(error, FormatsError::NotUtf8 { position: at(2, 14) });
        assert_eq!(error.code(), "RRS-E0002");
    }

    #[test]
    fn depth_limit_is_e5004_at_the_bracket_past_it() {
        let limits = ImportLimits {
            max_json_depth: 3,
            ..ImportLimits::default()
        };
        assert!(read_json(b"[[{\"a\": 1}]]", &limits).is_ok());
        let error = read_json(b"[[{\"a\": [1]}]]", &limits).unwrap_err();
        assert_eq!(
            error,
            FormatsError::TooDeep {
                position: at(1, 9),
                limit_depth: 3
            }
        );
        assert_eq!(error.code(), "RRS-E5004");
        // A deep document under the default limit stops cleanly instead of overflowing the stack.
        let deep = "[".repeat(100_000);
        assert_eq!(read(&deep).unwrap_err().code(), "RRS-E5004");
    }

    #[test]
    fn value_count_limit_is_e5006_before_the_tree_grows() {
        let limits = ImportLimits {
            max_json_values: 4,
            ..ImportLimits::default()
        };
        // The array and its three numbers are four values; nested containers count too.
        assert!(read_json(b"[1, 2, 3]", &limits).is_ok());
        assert!(read_json(br#"{"a": [{}]}"#, &limits).is_ok());
        let error = read_json(b"[1, 2, 3, 4]", &limits).unwrap_err();
        assert_eq!(
            error,
            FormatsError::TooManyValues {
                position: at(1, 11),
                limit_count: 4
            }
        );
        assert_eq!(error.code(), "RRS-E5006");
        let nested = read_json(br#"{"a": [{"b": [null]}]}"#, &limits).unwrap_err();
        assert_eq!(nested.code(), "RRS-E5006");
        // A document of tiny values under the default limits stops at the count, not at the end of its bytes.
        let defaults = ImportLimits::default();
        let value_limit = defaults.max_json_values as usize;
        let mut tiny_values = "[".to_string();
        tiny_values.push_str(&"0,".repeat(value_limit));
        tiny_values.push('0');
        tiny_values.push(']');
        let error = read(&tiny_values).unwrap_err();
        assert_eq!(error.code(), "RRS-E5006");
    }

    #[test]
    fn size_limit_is_e5003_before_parsing() {
        let limits = ImportLimits {
            max_file_bytes: 4,
            ..ImportLimits::default()
        };
        assert!(read_json(b"[12]", &limits).is_ok());
        let error = read_json(b"[123]", &limits).unwrap_err();
        assert_eq!(error.code(), "RRS-E5003");
    }

    #[test]
    fn values_come_out_right() {
        let text = r#"{"s": "a\"b\\c\/d\b\f\n\r\t\u00e9\ud83d\ude00", "t": true, "f": false, "n": null,
                       "big": 18446744073709551615, "low": -9223372036854775808, "huge": 18446744073709551616,
                       "float": 1.5e3, "zero": 0, "nested": [[], {}, [{}]]}"#;
        let parsed = read(text).unwrap();
        assert_eq!(parsed.value["s"], json!("a\"b\\c/d\u{8}\u{c}\n\r\té😀"));
        assert_eq!(parsed.value["big"], json!(u64::MAX));
        assert_eq!(parsed.value["low"], json!(i64::MIN));
        assert_eq!(parsed.value["huge"], json!(18446744073709551616.0));
        assert_eq!(parsed.value["float"], json!(1500.0));
        assert_eq!(parsed.value["nested"], json!([[], {}, [{}]]));
    }

    // Every document the strict reader accepts, serde_json reads to the same value. The cases cover each value
    // kind, number edge and escape.
    #[test]
    fn accepted_documents_equal_serde_json() {
        let documents = [
            "0",
            "-0",
            "-0.0",
            "1E2",
            "1e-2",
            "0.1",
            "123456789012345678901234567890",
            "-9223372036854775809",
            "[1.7976931348623157e308, 5e-324, 2.2250738585072014e-308]",
            r#""\u0000 \u001f \u007f \u2028 \u00e9 é""#,
            r#"{"z": 1, "a": [true, false, null], "m": {"": {"": ""}}}"#,
            " \t\r\n[ ] ",
        ];
        for text in documents {
            let ours = read(text).unwrap().value;
            let theirs: Value = serde_json::from_str(text).unwrap();
            assert_eq!(ours, theirs, "{text}");
        }
    }

    // Seeded mutations of a sample document (in-house, no fuzzing crate in unit tests): the reader never panics,
    // and whatever it accepts, serde_json accepts with the same value. The fuzz target does the same at scale.
    #[test]
    fn seeded_mutations_never_panic() {
        let sample = br#"{"type": "rastai.model", "schema_version": 1, "inputs": [{"range": [0, 5], "x": -1.5e-3}],
                          "name": "caf\u00e9", "flags": [true, false, null], "nested": {"a": {"b": [[]]}}}"#;
        let mut seed: u64 = 0x5EED_0001;
        for _ in 0..5_000 {
            let mut bytes = sample.to_vec();
            for _ in 0..1 + (next_random(&mut seed) % 4) {
                mutate(&mut bytes, &mut seed);
            }
            if let Ok(parsed) = read_json(&bytes, &ImportLimits::default()) {
                let theirs: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(parsed.value, theirs, "{:?}", String::from_utf8_lossy(&bytes));
            }
        }
    }

    const MUTATION_BYTES: &[u8] = b"{}[]\":,-.eE0123456789 \\u\"tfnNI\xFF\x00";

    fn mutate(bytes: &mut Vec<u8>, seed: &mut u64) {
        let index = (next_random(seed) as usize) % (bytes.len() + 1);
        let byte = MUTATION_BYTES[(next_random(seed) as usize) % MUTATION_BYTES.len()];
        match next_random(seed) % 3 {
            0 if index < bytes.len() => bytes[index] = byte,
            1 if index < bytes.len() => {
                bytes.remove(index);
            }
            _ => bytes.insert(index, byte),
        }
    }
}
