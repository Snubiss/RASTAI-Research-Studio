//! JSON-RPC 2.0 messages as MCP uses them (layout: rust-control-plane.md 7.2): parsing untrusted text into a
//! message, writing a message as one line, and the error replies that carry a catalog code.
//!
//! MCP input is untrusted, so `parse_message` is strict and refuses rather than guesses:
//! - text over `MAX_MESSAGE_BYTES` is refused before any parsing;
//! - the JSON goes through rastai-formats' strict reader (no duplicate keys, no comments, depth and value caps);
//! - batch arrays are refused (MCP removed them in 2025-06-18);
//! - `jsonrpc` must be `"2.0"`, ids are integers or strings (MCP forbids `null` request ids), `params` is an object,
//!   and a member JSON-RPC doesn't define is refused, so one text can't read as two different messages.
//!
//! Every refusal is `RRS-C3001` with the JSON-RPC code the reply carries and, when it could be read, the id.

use rastai_diag::diagnostic::Diagnostic;
use rastai_diag::render_json::render_json;
use rastai_formats::error::FormatsError;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde_json::{Map, Value};

use crate::error::{McpError, error_diagnostic, malformed_message};

/// Checked before parsing: MCP input is untrusted, and no tool's arguments come near this.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

// JSON-RPC 2.0's error codes (its section 5.1).
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// The first code of JSON-RPC's "server error" range: a Rastai diagnostic, which the error's `data` holds.
pub const RASTAI_ERROR: i64 = -32000;

const JSONRPC_VERSION: &str = "2.0";
/// The members JSON-RPC 2.0 defines for a message, and for its `error` object.
const MESSAGE_MEMBERS: &[&str] = &["jsonrpc", "id", "method", "params", "result", "error"];
const ERROR_MEMBERS: &[&str] = &["code", "message", "data"];

/// A request id. MCP allows integers and strings, never `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonRpcId {
    Number(i64),
    Text(String),
}

/// One JSON-RPC 2.0 message. An absent `params` or `data` is `Value::Null`.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcMessage {
    Request {
        id: JsonRpcId,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: JsonRpcId,
        result: Value,
    },
    /// `id` is None when the failed message's id couldn't be read (JSON-RPC then writes `null`).
    Error {
        id: Option<JsonRpcId>,
        code: i64,
        message: String,
        data: Value,
    },
}

/// The `id` member as the message has it: missing, `null`, or an id.
enum IdMember {
    Missing,
    Null,
    Present(JsonRpcId),
}

/// Reads one message from untrusted text. Anything else is `RRS-C3001`.
pub fn parse_message(text: &str) -> Result<JsonRpcMessage, McpError> {
    if text.len() > MAX_MESSAGE_BYTES {
        let reason = format!(
            "the message is {} bytes, over the {MAX_MESSAGE_BYTES}-byte limit",
            text.len()
        );
        return Err(malformed_message(&reason, INVALID_REQUEST, None));
    }
    match read_strict_json(text)? {
        Value::Object(object) => message_from_object(&object),
        Value::Array(_) => Err(malformed_message(
            "batch arrays aren't supported",
            INVALID_REQUEST,
            None,
        )),
        _ => Err(malformed_message(
            "a message must be a JSON object",
            INVALID_REQUEST,
            None,
        )),
    }
}

/// The message as one line of compact JSON: JSON escapes every newline inside a string, so the text never holds one.
pub fn message_json(message: &JsonRpcMessage) -> String {
    message_value(message).to_string()
}

/// The error reply for a refused message. A `MalformedMessage` that could read its id answers to that id; any other
/// error answers to `id`, the request being answered.
pub fn error_reply(id: Option<JsonRpcId>, error: &McpError) -> JsonRpcMessage {
    let (reply_id, json_rpc_code) = match error {
        McpError::MalformedMessage {
            id: Some(own_id),
            json_rpc_code,
            ..
        } => (Some(own_id.clone()), *json_rpc_code),
        McpError::MalformedMessage { json_rpc_code, .. } => (id, *json_rpc_code),
        McpError::NoToken | McpError::RandomUnavailable { .. } => (id, RASTAI_ERROR),
    };
    diagnostic_reply(reply_id, json_rpc_code, &error_diagnostic(error))
}

/// A JSON-RPC error carrying a diagnostic: its message as the error's message, and the whole diagnostic (code,
/// fix, path) as `data`, so a client or an agent can act on the code.
pub fn diagnostic_reply(id: Option<JsonRpcId>, json_rpc_code: i64, diagnostic: &Diagnostic) -> JsonRpcMessage {
    JsonRpcMessage::Error {
        id,
        code: json_rpc_code,
        message: diagnostic.message.clone(),
        data: render_json(diagnostic),
    }
}

fn read_strict_json(text: &str) -> Result<Value, McpError> {
    let limits = ImportLimits {
        max_file_bytes: MAX_MESSAGE_BYTES as u64,
        ..ImportLimits::default()
    };
    match read_json(text.as_bytes(), &limits) {
        Ok(parsed) => Ok(parsed.value),
        Err(error) => Err(malformed_message(&json_problem(&error), PARSE_ERROR, None)),
    }
}

/// What the strict reader refused, in words for the catalog message's `{reason}`.
fn json_problem(error: &FormatsError) -> String {
    match error {
        FormatsError::JsonSyntax { position, expected } => format!(
            "not JSON at line {}, column {}: expected {expected}",
            position.line, position.column
        ),
        FormatsError::DuplicateKey { pointer, .. } => format!("the key at {pointer} appears twice"),
        FormatsError::NotANumber { position } => format!(
            "a number JSON can't hold at line {}, column {}",
            position.line, position.column
        ),
        FormatsError::TooDeep { limit_depth, .. } => format!("nested deeper than {limit_depth} levels"),
        FormatsError::TooManyValues { limit_count, .. } => format!("more than {limit_count} values"),
        other => format!("not JSON ({})", other.code()),
    }
}

fn message_from_object(object: &Map<String, Value>) -> Result<JsonRpcMessage, McpError> {
    let id_member = read_id(object)?;
    let known_id = match &id_member {
        IdMember::Present(id) => Some(id.clone()),
        IdMember::Missing | IdMember::Null => None,
    };
    check_members(object, &known_id)?;
    if object.get("jsonrpc") != Some(&Value::String(JSONRPC_VERSION.to_string())) {
        return Err(malformed_message(
            "\"jsonrpc\" must be \"2.0\"",
            INVALID_REQUEST,
            known_id,
        ));
    }
    if object.contains_key("method") {
        return call_from_object(object, id_member);
    }
    if object.contains_key("result") {
        return response_from_object(object, id_member);
    }
    if object.contains_key("error") {
        return error_from_object(object, id_member);
    }
    Err(malformed_message(
        "a message needs \"method\", \"result\" or \"error\"",
        INVALID_REQUEST,
        known_id,
    ))
}

fn read_id(object: &Map<String, Value>) -> Result<IdMember, McpError> {
    let Some(value) = object.get("id") else {
        return Ok(IdMember::Missing);
    };
    match value {
        Value::Null => Ok(IdMember::Null),
        Value::String(text) => Ok(IdMember::Present(JsonRpcId::Text(text.clone()))),
        Value::Number(number) => match number.as_i64() {
            Some(whole) => Ok(IdMember::Present(JsonRpcId::Number(whole))),
            None => Err(malformed_message(
                "\"id\" must be a whole number that fits 64 bits, or a string",
                INVALID_REQUEST,
                None,
            )),
        },
        _ => Err(malformed_message(
            "\"id\" must be a whole number or a string",
            INVALID_REQUEST,
            None,
        )),
    }
}

/// Refuses a member JSON-RPC doesn't define, and a message that mixes a call with a reply.
fn check_members(object: &Map<String, Value>, known_id: &Option<JsonRpcId>) -> Result<(), McpError> {
    for key in object.keys() {
        if !MESSAGE_MEMBERS.contains(&key.as_str()) {
            return Err(malformed_message(
                &format!("unknown member {key:?}"),
                INVALID_REQUEST,
                known_id.clone(),
            ));
        }
    }
    let kinds = ["method", "result", "error"]
        .iter()
        .filter(|kind| object.contains_key(**kind))
        .count();
    if kinds > 1 {
        return Err(malformed_message(
            "a message has only one of \"method\", \"result\" and \"error\"",
            INVALID_REQUEST,
            known_id.clone(),
        ));
    }
    if object.contains_key("params") && !object.contains_key("method") {
        return Err(malformed_message(
            "only a request or notification has \"params\"",
            INVALID_REQUEST,
            known_id.clone(),
        ));
    }
    Ok(())
}

/// A request (with an id) or a notification (without one).
fn call_from_object(object: &Map<String, Value>, id_member: IdMember) -> Result<JsonRpcMessage, McpError> {
    let known_id = match &id_member {
        IdMember::Present(id) => Some(id.clone()),
        IdMember::Missing | IdMember::Null => None,
    };
    let Some(Value::String(method)) = object.get("method") else {
        return Err(malformed_message(
            "\"method\" must be a string",
            INVALID_REQUEST,
            known_id,
        ));
    };
    let params = match object.get("params") {
        None => Value::Null,
        Some(Value::Object(members)) => Value::Object(members.clone()),
        Some(_) => {
            return Err(malformed_message(
                "\"params\" must be an object",
                INVALID_REQUEST,
                known_id,
            ));
        }
    };
    let method = method.clone();
    match id_member {
        IdMember::Present(id) => Ok(JsonRpcMessage::Request { id, method, params }),
        IdMember::Missing => Ok(JsonRpcMessage::Notification { method, params }),
        IdMember::Null => Err(malformed_message(
            "a request's \"id\" can't be null",
            INVALID_REQUEST,
            None,
        )),
    }
}

fn response_from_object(object: &Map<String, Value>, id_member: IdMember) -> Result<JsonRpcMessage, McpError> {
    let IdMember::Present(id) = id_member else {
        return Err(malformed_message("a response needs an \"id\"", INVALID_REQUEST, None));
    };
    let result = object.get("result").cloned().unwrap_or(Value::Null);
    Ok(JsonRpcMessage::Response { id, result })
}

fn error_from_object(object: &Map<String, Value>, id_member: IdMember) -> Result<JsonRpcMessage, McpError> {
    let id = match id_member {
        IdMember::Present(id) => Some(id),
        IdMember::Null => None,
        IdMember::Missing => {
            return Err(malformed_message(
                "an error needs an \"id\", which may be null",
                INVALID_REQUEST,
                None,
            ));
        }
    };
    let Some(Value::Object(error)) = object.get("error") else {
        return Err(malformed_message("\"error\" must be an object", INVALID_REQUEST, id));
    };
    if let Some(key) = error.keys().find(|key| !ERROR_MEMBERS.contains(&key.as_str())) {
        return Err(malformed_message(
            &format!("unknown member {key:?} in \"error\""),
            INVALID_REQUEST,
            id,
        ));
    }
    let Some(code) = error.get("code").and_then(Value::as_i64) else {
        return Err(malformed_message(
            "\"error.code\" must be a whole number",
            INVALID_REQUEST,
            id,
        ));
    };
    let Some(Value::String(message)) = error.get("message") else {
        return Err(malformed_message(
            "\"error.message\" must be a string",
            INVALID_REQUEST,
            id,
        ));
    };
    let data = error.get("data").cloned().unwrap_or(Value::Null);
    let message = message.clone();
    Ok(JsonRpcMessage::Error {
        id,
        code,
        message,
        data,
    })
}

fn message_value(message: &JsonRpcMessage) -> Value {
    let mut object = Map::new();
    object.insert("jsonrpc".to_string(), Value::String(JSONRPC_VERSION.to_string()));
    match message {
        JsonRpcMessage::Request { id, method, params } => {
            object.insert("id".to_string(), id_value(id));
            insert_call(&mut object, method, params);
        }
        JsonRpcMessage::Notification { method, params } => insert_call(&mut object, method, params),
        JsonRpcMessage::Response { id, result } => {
            object.insert("id".to_string(), id_value(id));
            object.insert("result".to_string(), result.clone());
        }
        JsonRpcMessage::Error {
            id,
            code,
            message,
            data,
        } => {
            let id = id.as_ref().map_or(Value::Null, id_value);
            object.insert("id".to_string(), id);
            object.insert("error".to_string(), error_value(*code, message, data));
        }
    }
    Value::Object(object)
}

fn insert_call(object: &mut Map<String, Value>, method: &str, params: &Value) {
    object.insert("method".to_string(), Value::String(method.to_string()));
    if !params.is_null() {
        object.insert("params".to_string(), params.clone());
    }
}

fn error_value(code: i64, message: &str, data: &Value) -> Value {
    let mut error = Map::new();
    error.insert("code".to_string(), Value::from(code));
    error.insert("message".to_string(), Value::String(message.to_string()));
    if !data.is_null() {
        error.insert("data".to_string(), data.clone());
    }
    Value::Object(error)
}

fn id_value(id: &JsonRpcId) -> Value {
    match id {
        JsonRpcId::Number(number) => Value::from(*number),
        JsonRpcId::Text(text) => Value::String(text.clone()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The id of a message, if it has one (requests, responses and errors that could read theirs).
    fn message_id(message: &JsonRpcMessage) -> Option<&JsonRpcId> {
        match message {
            JsonRpcMessage::Request { id, .. } | JsonRpcMessage::Response { id, .. } => Some(id),
            JsonRpcMessage::Error { id, .. } => id.as_ref(),
            JsonRpcMessage::Notification { .. } => None,
        }
    }

    fn refusal(text: &str) -> (String, i64, Option<JsonRpcId>) {
        match parse_message(text).unwrap_err() {
            McpError::MalformedMessage {
                reason,
                json_rpc_code,
                id,
            } => (reason, json_rpc_code, id),
            other => panic!("expected a malformed message, got {other:?}"),
        }
    }

    #[test]
    fn ids_are_numbers_or_strings() {
        let number = parse_message(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap();
        let text = parse_message(r#"{"jsonrpc":"2.0","id":"a-7","method":"ping"}"#).unwrap();
        let negative = parse_message(r#"{"jsonrpc":"2.0","id":-9223372036854775808,"result":{}}"#).unwrap();
        assert_eq!(message_id(&number), Some(&JsonRpcId::Number(7)));
        assert_eq!(message_id(&text), Some(&JsonRpcId::Text("a-7".to_string())));
        assert_eq!(message_id(&negative), Some(&JsonRpcId::Number(i64::MIN)));
    }

    #[test]
    fn ids_that_are_not_whole_numbers_or_strings_are_refused() {
        for id in ["1.5", "true", "[1]", "{}", "18446744073709551615"] {
            let text = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#);
            let (_, code, reply_id) = refusal(&text);
            assert_eq!(code, INVALID_REQUEST, "{id}");
            assert_eq!(reply_id, None, "{id}");
        }
        let (reason, _, _) = refusal(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#);
        assert_eq!(reason, "a request's \"id\" can't be null");
    }

    #[test]
    fn batch_arrays_are_refused() {
        let (reason, code, _) = refusal(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        assert_eq!(reason, "batch arrays aren't supported");
        assert_eq!(code, INVALID_REQUEST);
    }

    #[test]
    fn text_that_is_not_json_is_a_parse_error() {
        let (reason, code, _) = refusal("{\"jsonrpc\":");
        assert_eq!(code, PARSE_ERROR);
        assert!(reason.starts_with("not JSON at line 1, column 12"), "{reason}");
        let (reason, code, _) = refusal(r#"{"jsonrpc":"2.0","id":1,"id":2,"method":"ping"}"#);
        assert_eq!(code, PARSE_ERROR);
        assert_eq!(reason, "the key at /id appears twice");
    }

    #[test]
    fn oversize_text_is_refused_before_parsing() {
        // Not JSON at all: a parse would say "not JSON", so the size message shows the size check came first.
        let text = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let (reason, code, _) = refusal(&text);
        assert_eq!(
            reason,
            format!("the message is {} bytes, over the 4194304-byte limit", text.len())
        );
        assert_eq!(code, INVALID_REQUEST);
    }

    #[test]
    fn a_text_of_exactly_the_limit_is_parsed() {
        let padding = " ".repeat(MAX_MESSAGE_BYTES - r#"{"jsonrpc":"2.0","method":"x"}"#.len());
        let text = format!(r#"{{"jsonrpc":"2.0","method":"x"}}{padding}"#);
        assert_eq!(text.len(), MAX_MESSAGE_BYTES);
        assert!(parse_message(&text).is_ok());
    }

    #[test]
    fn wrong_members_are_refused_with_the_id_when_it_was_read() {
        let cases = [
            (r#"{"id":1,"method":"ping"}"#, "\"jsonrpc\" must be \"2.0\""),
            (
                r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
                "\"jsonrpc\" must be \"2.0\"",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"ping","extra":1}"#,
                "unknown member \"extra\"",
            ),
            (r#"{"jsonrpc":"2.0","id":1,"method":7}"#, "\"method\" must be a string"),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"x","params":[1]}"#,
                "\"params\" must be an object",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"x","params":null}"#,
                "\"params\" must be an object",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"x","result":{}}"#,
                "a message has only one of \"method\", \"result\" and \"error\"",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"result":{},"params":{}}"#,
                "only a request or notification has \"params\"",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1}"#,
                "a message needs \"method\", \"result\" or \"error\"",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":1.5,"message":"m"}}"#,
                "\"error.code\" must be a whole number",
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"m","x":0}}"#,
                "unknown member \"x\" in \"error\"",
            ),
        ];
        for (text, expected_reason) in cases {
            let (reason, code, id) = refusal(text);
            assert_eq!(reason, expected_reason, "{text}");
            assert_eq!(code, INVALID_REQUEST, "{text}");
            assert_eq!(id, Some(JsonRpcId::Number(1)), "{text}");
        }
    }

    #[test]
    fn an_error_may_have_a_null_id_but_not_a_missing_one() {
        let message = parse_message(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"m"}}"#).unwrap();
        assert_eq!(message_id(&message), None);
        let (reason, _, _) = refusal(r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"m"}}"#);
        assert_eq!(reason, "an error needs an \"id\", which may be null");
    }

    #[test]
    fn every_kind_round_trips_through_one_line() {
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"a\nb","arguments":{}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":"x","result":null}),
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"m","data":{"code":"RRS-C3001"}}}),
        ];
        for value in messages {
            let message = parse_message(&value.to_string()).unwrap();
            let line = message_json(&message);
            assert!(!line.contains('\n'), "{line}");
            assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), value);
            assert_eq!(parse_message(&line).unwrap(), message);
        }
    }

    #[test]
    fn an_error_reply_carries_the_code_and_the_read_id() {
        let error = parse_message(r#"{"jsonrpc":"2.0","id":4,"method":"x","params":[]}"#).unwrap_err();
        let reply = error_reply(Some(JsonRpcId::Number(9)), &error);
        let value: Value = serde_json::from_str(&message_json(&reply)).unwrap();
        assert_eq!(value["id"], json!(4));
        assert_eq!(value["error"]["code"], json!(INVALID_REQUEST));
        assert_eq!(value["error"]["data"]["code"], json!("RRS-C3001"));
        let no_token = error_reply(Some(JsonRpcId::Number(9)), &McpError::NoToken);
        let value: Value = serde_json::from_str(&message_json(&no_token)).unwrap();
        assert_eq!(value["id"], json!(9));
        assert_eq!(value["error"]["code"], json!(RASTAI_ERROR));
        assert_eq!(value["error"]["data"]["code"], json!("RRS-C3002"));
    }
}
