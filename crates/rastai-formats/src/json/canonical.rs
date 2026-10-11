//! Canonical JSON: the exact bytes a content hash is taken over (contracts 5.1). One value has one canonical text,
//! whatever spacing, key order or number spelling its file used.
//!
//! The rules are contracts 5.1 "Canonical JSON". They are RFC 8785's (the JSON Canonicalization Scheme) with two
//! deliberate differences, listed there: keys sort by code point, not UTF-16 code unit, and integers outside ±2^53
//! are written exactly, so integer 2^60 and float 2^60 have different text. In short:
//! - no whitespace between tokens;
//! - object keys sorted by code point, which is the byte order of their UTF-8;
//! - strings as UTF-8, escaping only `"`, `\` and control characters: `\b \t \n \f \r` in short form, the others
//!   as `\u00XX` with lowercase hex;
//! - integers that serde_json holds as u64 or i64 written as plain decimals, exactly;
//! - other numbers written the way JavaScript's `Number.prototype.toString` writes them: the shortest digits that
//!   read back to the same f64, plain decimals from 1e-6 up to 1e21, exponent form (`1e+21`, `1e-7`) outside
//!   that, and `-0` as `0`. So `1`, `1.0` and `1e0` all become `1`.
//!
//! Content hashes are computed only here, so JavaScript and Python never have to reproduce these rules.

use serde_json::{Number, Value};

/// JavaScript writes numbers in plain decimal while the decimal point sits within 21 digits of the first digit.
const PLAIN_DECIMAL_MAX_POINT: i32 = 21;
/// …and down to 6 zeros after the point (`0.000001`); smaller numbers use the exponent form.
const PLAIN_DECIMAL_MIN_POINT: i32 = -6;

/// The canonical text of a JSON value.
pub fn canonical_json(value: &Value) -> String {
    let mut output = String::new();
    write_value(&mut output, value);
    output
}

fn write_value(output: &mut String, value: &Value) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(true) => output.push_str("true"),
        Value::Bool(false) => output.push_str("false"),
        Value::Number(number) => output.push_str(&number_text(number)),
        Value::String(text) => write_string(output, text),
        Value::Array(elements) => write_array(output, elements),
        Value::Object(members) => write_object(output, members),
    }
}

fn write_array(output: &mut String, elements: &[Value]) {
    output.push('[');
    for (index, element) in elements.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_value(output, element);
    }
    output.push(']');
}

/// Keys are sorted here, not taken in map order: if any crate in a build turned on serde_json's
/// `preserve_order` feature, maps would iterate in insertion order and every content hash would change. No test
/// can catch the sort going missing, because this build's maps are already sorted (`cargo deny` would have to let
/// the feature in first); keep it.
fn write_object(output: &mut String, members: &serde_json::Map<String, Value>) {
    let mut keys: Vec<&String> = members.keys().collect();
    keys.sort();
    output.push('{');
    for (index, key) in keys.into_iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_string(output, key);
        output.push(':');
        if let Some(member) = members.get(key) {
            write_value(output, member);
        }
    }
    output.push('}');
}

fn write_string(output: &mut String, text: &str) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\u{000C}' => output.push_str("\\f"),
            '\r' => output.push_str("\\r"),
            control if (control as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", control as u32)),
            other => output.push(other),
        }
    }
    output.push('"');
}

fn number_text(number: &Number) -> String {
    if let Some(unsigned) = number.as_u64() {
        return unsigned.to_string();
    }
    if let Some(signed) = number.as_i64() {
        return signed.to_string();
    }
    // serde_json numbers are always one of the three, and never NaN or infinite.
    float_text(number.as_f64().unwrap_or(0.0))
}

/// JavaScript's `Number.prototype.toString` for a finite f64 (ECMA-262, Number::toString). Rust's `{:e}`
/// already gives the shortest round-trip digits; this only places the decimal point the way JavaScript does.
fn float_text(number: f64) -> String {
    if number == 0.0 {
        return "0".to_string();
    }
    let sign = if number < 0.0 { "-" } else { "" };
    let (digits, exponent) = shortest_digits(number.abs());
    // `point` is where the decimal point goes, counted from the left of the digits: 1.5e3 has digits "15", point 4.
    let point = exponent + 1;
    let digit_count = digits.len() as i32;
    let body = if digit_count <= point && point <= PLAIN_DECIMAL_MAX_POINT {
        format!("{digits}{}", "0".repeat((point - digit_count) as usize))
    } else if 0 < point && point <= PLAIN_DECIMAL_MAX_POINT {
        let (whole, fraction) = digits.split_at(point as usize);
        format!("{whole}.{fraction}")
    } else if PLAIN_DECIMAL_MIN_POINT < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        exponent_form(&digits, exponent)
    };
    format!("{sign}{body}")
}

/// `1e+21`, `1.5e-7`: one digit, the rest after a point, and the exponent with its sign.
fn exponent_form(digits: &str, exponent: i32) -> String {
    let (first, rest) = digits.split_at(1);
    let point = if rest.is_empty() { "" } else { "." };
    let exponent_sign = if exponent >= 0 { "+" } else { "-" };
    format!("{first}{point}{rest}e{exponent_sign}{}", exponent.abs())
}

/// The shortest decimal digits of a positive finite number and the power of ten of the first digit:
/// 1500.0 → ("15", 3), 0.001 → ("1", -3).
fn shortest_digits(number: f64) -> (String, i32) {
    let scientific = format!("{number:e}");
    let (mantissa, exponent_text) = scientific.split_once('e').unwrap_or((scientific.as_str(), "0"));
    let digits: String = mantissa.chars().filter(|character| *character != '.').collect();
    let exponent = exponent_text.parse().unwrap_or(0);
    (digits, exponent)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn float(number: f64) -> String {
        canonical_json(&json!(number))
    }

    #[test]
    fn keys_sorted_by_code_point_at_every_level_with_no_whitespace() {
        let value = json!({
            "b": 1, "a": {"z": [3, {"y": 1, "x": 2}], "A": null}, "é": true, "_": false, "$schema": "s", "10": 0, "9": 0
        });
        assert_eq!(
            canonical_json(&value),
            r#"{"$schema":"s","10":0,"9":0,"_":false,"a":{"A":null,"z":[3,{"x":2,"y":1}]},"b":1,"é":true}"#
        );
    }

    #[test]
    fn spacing_and_key_order_of_the_source_do_not_matter() {
        let first: Value = serde_json::from_str("{ \"b\" : [1, 2] ,\n \"a\" : {\"d\": 1, \"c\": 2} }").unwrap();
        let second: Value = serde_json::from_str(r#"{"a":{"c":2,"d":1},"b":[1,2]}"#).unwrap();
        assert_eq!(canonical_json(&first), canonical_json(&second));
        assert_eq!(canonical_json(&first), r#"{"a":{"c":2,"d":1},"b":[1,2]}"#);
    }

    #[test]
    fn integers_are_exact() {
        assert_eq!(canonical_json(&json!(0)), "0");
        assert_eq!(canonical_json(&json!(u64::MAX)), "18446744073709551615");
        assert_eq!(canonical_json(&json!(i64::MIN)), "-9223372036854775808");
        assert_eq!(canonical_json(&json!(-1)), "-1");
    }

    // The number vectors of RFC 8785 appendix B, plus the boundaries of the plain-decimal range. Inputs are text,
    // as in the RFC, read with Rust's correctly rounded parser.
    #[test]
    fn floats_follow_javascript() {
        let cases = [
            ("0.0", "0"),
            ("-0.0", "0"),
            ("1.0", "1"),
            ("-1.0", "-1"),
            ("4.50", "4.5"),
            ("2e-3", "0.002"),
            ("0.000001", "0.000001"),
            ("1e-7", "1e-7"),
            ("1.5e-7", "1.5e-7"),
            ("123.456", "123.456"),
            ("1e20", "100000000000000000000"),
            ("1e21", "1e+21"),
            ("1.5e21", "1.5e+21"),
            ("1E30", "1e+30"),
            ("333333333.33333329", "333333333.3333333"),
            ("5e-324", "5e-324"),
            ("-5e-324", "-5e-324"),
            ("1.7976931348623157e308", "1.7976931348623157e+308"),
            ("-1.7976931348623157e308", "-1.7976931348623157e+308"),
            ("9007199254740992.0", "9007199254740992"),
            ("295147905179352825856.0", "295147905179352830000"),
            ("0.1", "0.1"),
            ("1e23", "1e+23"),
        ];
        for (text, expected) in cases {
            assert_eq!(float(text.parse().unwrap()), expected, "{text}");
        }
    }

    #[test]
    fn integer_and_float_spellings_of_one_value_agree() {
        let spellings = ["1", "1.0", "1e0", "10e-1", "0.1e1"];
        for text in spellings {
            let value: Value = serde_json::from_str(text).unwrap();
            assert_eq!(canonical_json(&value), "1", "{text}");
        }
    }

    #[test]
    fn floats_read_back_to_the_same_number() {
        for number in [
            0.1,
            1.0 / 3.0,
            2.0f64.powi(-1074),
            123456.789e-30,
            6.02214076e23,
            f64::MAX,
            f64::MIN_POSITIVE,
        ] {
            let text = float(number);
            assert_eq!(text.parse::<f64>().unwrap(), number, "{text}");
        }
    }

    #[test]
    fn strings_escape_only_what_json_requires() {
        let value = json!("q\" b\\ \u{8}\t\n\u{c}\r \u{0} \u{1f} \u{7f} é 😀 \u{2028} /");
        assert_eq!(
            canonical_json(&value),
            "\"q\\\" b\\\\ \\b\\t\\n\\f\\r \\u0000 \\u001f \u{7f} é 😀 \u{2028} /\""
        );
    }

    #[test]
    fn literals_and_empty_containers() {
        assert_eq!(
            canonical_json(&json!([null, true, false, [], {}, ""])),
            r#"[null,true,false,[],{},""]"#
        );
    }
}
