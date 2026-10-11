//! JSON pointers (RFC 6901): `/inputs/1/range`. Every diagnostic path, position and patch operation names a value
//! this way, so escaping lives in one place.
//!
//! A pointer is "" (the whole document) or a list of tokens, each after a `/`. Inside a token `~` is written `~0`
//! and `/` is written `~1`.

use serde_json::Value;

/// The pointer of the whole document (RFC 6901: the empty string).
pub(crate) const ROOT_POINTER: &str = "";

const SEPARATOR: char = '/';
const ESCAPE: char = '~';
const ESCAPED_TILDE: &str = "~0";
const ESCAPED_SLASH: &str = "~1";

/// Escapes one object key or array index so it can be a pointer token.
pub fn escape_pointer_token(token: &str) -> String {
    // `~` first, so the `~` that `~1` introduces isn't escaped again.
    token.replace(ESCAPE, ESCAPED_TILDE).replace(SEPARATOR, ESCAPED_SLASH)
}

/// The pointer of a child: `join_pointer("/inputs", "1")` is `/inputs/1`. The token is escaped here.
pub fn join_pointer(parent: &str, token: &str) -> String {
    let escaped = escape_pointer_token(token);
    let mut pointer = String::with_capacity(parent.len() + 1 + escaped.len());
    pointer.push_str(parent);
    pointer.push(SEPARATOR);
    pointer.push_str(&escaped);
    pointer
}

/// The value a pointer names, or None when the pointer is malformed or names nothing.
pub fn lookup_pointer<'a>(document: &'a Value, pointer: &str) -> Option<&'a Value> {
    let tokens = split_pointer(pointer)?;
    let mut current = document;
    for token in &tokens {
        current = child_value(current, token)?;
    }
    Some(current)
}

/// The unescaped tokens of a pointer: `/a~1b/0` → `["a/b", "0"]`, "" → `[]`. None when the text isn't a pointer:
/// it doesn't start with `/`, or a `~` is followed by anything but `0` or `1`.
pub(crate) fn split_pointer(pointer: &str) -> Option<Vec<String>> {
    if pointer.is_empty() {
        return Some(Vec::new());
    }
    let rest = pointer.strip_prefix(SEPARATOR)?;
    rest.split(SEPARATOR).map(unescape_pointer_token).collect()
}

/// An array index token: decimal digits with no leading zero (RFC 6901 section 4). `-` and `01` are not indexes.
pub(crate) fn parse_array_index(token: &str) -> Option<usize> {
    let all_digits = !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit());
    let leading_zero = token.len() > 1 && token.starts_with('0');
    if !all_digits || leading_zero {
        return None;
    }
    token.parse().ok()
}

/// One step down: a member of an object or an element of an array.
fn child_value<'a>(parent: &'a Value, token: &str) -> Option<&'a Value> {
    match parent {
        Value::Object(members) => members.get(token),
        Value::Array(elements) => elements.get(parse_array_index(token)?),
        _ => None,
    }
}

/// `a~1b` → `a/b`, `m~0n` → `m~n`. None for a `~` followed by anything but `0` or `1`.
pub(crate) fn unescape_pointer_token(token: &str) -> Option<String> {
    let mut unescaped = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character != ESCAPE {
            unescaped.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => unescaped.push(ESCAPE),
            Some('1') => unescaped.push(SEPARATOR),
            _ => return None,
        }
    }
    Some(unescaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn escape_and_join() {
        assert_eq!(escape_pointer_token("a/b~c"), "a~1b~0c");
        assert_eq!(escape_pointer_token("~1"), "~01");
        assert_eq!(join_pointer("", "inputs"), "/inputs");
        assert_eq!(join_pointer("/inputs", "1"), "/inputs/1");
        assert_eq!(join_pointer("/a", ""), "/a/");
    }

    #[test]
    fn split_unescapes_and_refuses_bad_escapes() {
        assert_eq!(split_pointer(""), Some(vec![]));
        assert_eq!(
            split_pointer("/a~1b/~01"),
            Some(vec!["a/b".to_string(), "~1".to_string()])
        );
        assert_eq!(split_pointer("/"), Some(vec![String::new()]));
        assert_eq!(split_pointer("a"), None);
        assert_eq!(split_pointer("/a~2"), None);
        assert_eq!(split_pointer("/a~"), None);
    }

    #[test]
    fn lookup_follows_the_rfc_examples() {
        // RFC 6901 section 5.
        let document = json!({
            "foo": ["bar", "baz"], "": 0, "a/b": 1, "c%d": 2, "e^f": 3, "g|h": 4, "i\\j": 5, "k\"l": 6, " ": 7, "m~n": 8
        });
        let cases = [
            ("", document.clone()),
            ("/foo", json!(["bar", "baz"])),
            ("/foo/0", json!("bar")),
            ("/", json!(0)),
            ("/a~1b", json!(1)),
            ("/c%d", json!(2)),
            ("/e^f", json!(3)),
            ("/g|h", json!(4)),
            ("/i\\j", json!(5)),
            ("/k\"l", json!(6)),
            ("/ ", json!(7)),
            ("/m~0n", json!(8)),
        ];
        for (pointer, expected) in cases {
            assert_eq!(
                lookup_pointer(&document, pointer),
                Some(&expected),
                "pointer {pointer:?}"
            );
        }
    }

    #[test]
    fn lookup_misses() {
        let document = json!({"list": [1, 2], "number": 3});
        for pointer in [
            "/missing",
            "/list/2",
            "/list/-",
            "/list/01",
            "/number/0",
            "list",
            "/list/+1",
        ] {
            assert_eq!(lookup_pointer(&document, pointer), None, "pointer {pointer:?}");
        }
    }

    #[test]
    fn array_indexes() {
        assert_eq!(parse_array_index("0"), Some(0));
        assert_eq!(parse_array_index("12"), Some(12));
        for token in ["", "-", "01", "1a", "-1", "99999999999999999999999"] {
            assert_eq!(parse_array_index(token), None, "token {token:?}");
        }
    }
}
