//! The Connect protocol (layout: rust-control-plane.md 11.3; connectrpc.com/docs/protocol): content-type selection,
//! the unary bodies of `CommandService`, the Connect error envelope, and the stream envelopes `EventService.Subscribe`
//! (M1-28) writes. The only file that uses prost (layout 8.4).
//!
//! - **Unary** (`Execute`, `ListCommands`): a POST whose body is the message itself, no envelope, as
//!   `application/proto` (connect-es's default, the web app) or `application/json` (curl, scripts, the CLI), and an
//!   answer in the same content type. JSON goes through the hand-written codec, `connect_json.rs`.
//! - **Errors in the Connect envelope** are only the gate's refusals and a request that can't be read at all (a body
//!   that doesn't decode, a content type the service doesn't take). A command that fails is not one: it is
//!   `ExecuteResponse{ok: false, diagnostics}` with HTTP 200 (flows/platform.md 3.1). The envelope is
//!   `{"code": "unauthenticated", "message": ..., "details": [...]}` with the HTTP status of its code; its one detail
//!   is the catalog `Diagnostic`, as base64 protobuf (`value`) and as JSON (`debug`), so connect-es finds it with
//!   `findDetails` and curl shows it as text.
//! - **Streams** (`EventService.Subscribe`, binary only: `application/connect+proto`, layout 11.3): the request body
//!   is one enveloped message; the answer is HTTP 200 with each message in a 5-byte envelope (flags, then the length
//!   as a big-endian u32), and a last end-of-stream envelope (flag 2) whose JSON body is `{}` or
//!   `{"error": <the error envelope>}`. A refusal after the gate, such as `RRS-C0401`, is that last envelope.
//!   The body is fed from a channel through tokio-stream's `ReceiverStream`, which tonic re-exports (contracts 2.5:
//!   tonic is direct in this crate), so no stream crate of its own is needed.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::convert::Infallible;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use prost::Message;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse};
use rastai_proto::events::v1::{BusEvent, SubscribeRequest};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;

use crate::http::connect_json::{
    diagnostic_to_json, execute_request_from_json, execute_response_to_json, list_commands_request_from_json,
    list_commands_response_to_json,
};

const PROTO_CONTENT_TYPE: &str = "application/proto";
const JSON_CONTENT_TYPE: &str = "application/json";
/// The one content type of a Connect server stream the daemon serves (layout 11.3).
const STREAM_PROTO_CONTENT_TYPE: &str = "application/connect+proto";
/// What `Accept-Post` lists when a request's content type is refused: the unary services' two, or the stream's one.
pub(crate) const UNARY_CONTENT_TYPES: &str = "application/proto, application/json";
pub(crate) const STREAM_CONTENT_TYPES: &str = "application/connect+proto";
/// `application/json; charset=utf-8` is JSON: the parameters after `;` don't change the codec.
const CONTENT_TYPE_PARAMETERS_START: char = ';';
/// The response header that lists the content types a POST takes (RFC 7694); the http crate has no constant for it.
const ACCEPT_POST_HEADER: &str = "accept-post";
/// How a refusal names a request without a Content-Type.
const NO_CONTENT_TYPE: &str = "no Content-Type";

/// The full name of the message every error detail carries (connect-es's `findDetails(DiagnosticSchema)`).
const DIAGNOSTIC_TYPE_NAME: &str = "rastai.common.v1.Diagnostic";

// Connect's error codes for the refusals the daemon makes, each with its HTTP status (the Connect protocol's table).
const CONNECT_UNAUTHENTICATED: &str = "unauthenticated";
const CONNECT_PERMISSION_DENIED: &str = "permission_denied";
const CONNECT_INVALID_ARGUMENT: &str = "invalid_argument";
const CONNECT_INTERNAL: &str = "internal";
/// A missing, unknown, revoked or expired credential.
const UNAUTHENTICATED_CODES: &[&str] = &["RRS-C1002", "RRS-C1006", "RRS-C1007"];
/// A refused Host, Origin or scope.
const PERMISSION_DENIED_CODES: &[&str] = &["RRS-C1003", "RRS-C1004", "RRS-C1005"];
/// A body that isn't the message: `RRS-C0002`, or the `RRS-E` code the JSON codec refused it with.
const INVALID_ARGUMENT_CODE: &str = "RRS-C0002";
const FORMATS_CODE_PREFIX: &str = "RRS-E";

// The stream envelope: one flags byte, then the message length as a big-endian u32.
const ENVELOPE_HEADER_BYTES: usize = 5;
const MESSAGE_FLAGS: u8 = 0;
const END_OF_STREAM_FLAGS: u8 = 0b10;
const SUBSCRIBE_REQUEST_NAME: &str = "rastai.events.v1.SubscribeRequest";

/// Standard base64 (RFC 4648 section 4). Connect writes error details without padding, and readers accept both.
const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_GROUP_BYTES: usize = 3;
const BASE64_SIX_BITS: u32 = 0b11_1111;

/// One piece of a stream's body: an envelope's bytes. The error type is `Infallible` because a piece, once made, is
/// always sent; a stream that fails ends with its end-of-stream envelope instead.
pub(crate) type StreamChunk = Result<Vec<u8>, Infallible>;

/// How a unary body is encoded, from the request's Content-Type; the answer uses the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectCodec {
    Proto,
    Json,
}

/// The codec of a Content-Type header value, or None for one `CommandService` doesn't take. The media type is
/// compared without its parameters and without regard to case, as HTTP says.
pub(crate) fn codec_for_content_type(content_type: &str) -> Option<ConnectCodec> {
    let media_type = match content_type.split_once(CONTENT_TYPE_PARAMETERS_START) {
        Some((media_type, _parameters)) => media_type,
        None => content_type,
    };
    let media_type = media_type.trim();
    if media_type.eq_ignore_ascii_case(PROTO_CONTENT_TYPE) {
        return Some(ConnectCodec::Proto);
    }
    if media_type.eq_ignore_ascii_case(JSON_CONTENT_TYPE) {
        return Some(ConnectCodec::Json);
    }
    None
}

/// True for `application/connect+proto`, compared like `codec_for_content_type` compares.
pub(crate) fn is_stream_content_type(content_type: &str) -> bool {
    let media_type = match content_type.split_once(CONTENT_TYPE_PARAMETERS_START) {
        Some((media_type, _parameters)) => media_type,
        None => content_type,
    };
    media_type.trim().eq_ignore_ascii_case(STREAM_PROTO_CONTENT_TYPE)
}

/// A server stream's request: exactly one uncompressed envelope holding the message. Compression isn't offered, so
/// a compressed envelope, a cut-off one or anything after it is `RRS-C0002`.
pub(crate) fn decode_subscribe_request(body: &[u8]) -> Result<SubscribeRequest, Diagnostic> {
    let Some((header, message)) = body.split_first_chunk::<ENVELOPE_HEADER_BYTES>() else {
        return Err(undecodable_stream_body());
    };
    let flags = header[0];
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if flags != MESSAGE_FLAGS || message.len() != length {
        return Err(undecodable_stream_body());
    }
    SubscribeRequest::decode(message).map_err(|_| undecodable_stream_body())
}

/// One bus event in its stream envelope.
pub(crate) fn bus_event_envelope(event: &BusEvent) -> Vec<u8> {
    stream_message_envelope(&event.encode_to_vec())
}

/// HTTP 200 with a server stream's content type, its body the pieces the channel carries, in order, until the
/// sender is dropped.
pub(crate) fn stream_response(chunks: mpsc::Receiver<StreamChunk>) -> Response {
    let body = Body::from_stream(ReceiverStream::new(chunks));
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, STREAM_PROTO_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// A server stream refused before its first message: HTTP 200 and only the end-of-stream envelope with the error,
/// as the Connect protocol answers any stream that fails.
pub(crate) fn stream_refusal_response(refusal: &Diagnostic) -> Response {
    let body = end_of_stream_envelope(Some(refusal));
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, STREAM_PROTO_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

pub(crate) fn decode_execute_request(codec: ConnectCodec, body: &[u8]) -> Result<ExecuteRequest, Diagnostic> {
    match codec {
        ConnectCodec::Proto => {
            ExecuteRequest::decode(body).map_err(|_| undecodable_body("rastai.commands.v1.ExecuteRequest"))
        }
        ConnectCodec::Json => execute_request_from_json(body),
    }
}

pub(crate) fn encode_execute_response(codec: ConnectCodec, response: &ExecuteResponse) -> Vec<u8> {
    match codec {
        ConnectCodec::Proto => response.encode_to_vec(),
        ConnectCodec::Json => execute_response_to_json(response).into_bytes(),
    }
}

pub(crate) fn decode_list_commands_request(
    codec: ConnectCodec,
    body: &[u8],
) -> Result<ListCommandsRequest, Diagnostic> {
    match codec {
        ConnectCodec::Proto => {
            ListCommandsRequest::decode(body).map_err(|_| undecodable_body("rastai.commands.v1.ListCommandsRequest"))
        }
        ConnectCodec::Json => list_commands_request_from_json(body),
    }
}

pub(crate) fn encode_list_commands_response(codec: ConnectCodec, response: &ListCommandsResponse) -> Vec<u8> {
    match codec {
        ConnectCodec::Proto => response.encode_to_vec(),
        ConnectCodec::Json => list_commands_response_to_json(response).into_bytes(),
    }
}

/// A unary answer: HTTP 200 with the encoded message.
pub(crate) fn unary_response(codec: ConnectCodec, body: Vec<u8>) -> Response {
    let content_type = match codec {
        ConnectCodec::Proto => PROTO_CONTENT_TYPE,
        ConnectCodec::Json => JSON_CONTENT_TYPE,
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// A refusal in the Connect error envelope, with the HTTP status of its Connect code.
pub(crate) fn connect_error_response(refusal: &Diagnostic) -> Response {
    let (_code, status) = connect_code(&refusal.code);
    connect_error_with_status(refusal, status)
}

/// 415 for a Content-Type the service doesn't take, with `Accept-Post` naming the ones it does (`accepted`:
/// `UNARY_CONTENT_TYPES` or `STREAM_CONTENT_TYPES`). The body is the Connect error envelope all the same, so a client
/// reads one shape for every refusal.
pub(crate) fn unsupported_content_type_response(content_type: Option<&str>, accepted: &'static str) -> Response {
    let found = format!("Content-Type {}", content_type.unwrap_or(NO_CONTENT_TYPE));
    let expected = format!("Content-Type {accepted}");
    let refusal = diagnostic(INVALID_ARGUMENT_CODE, &[("found", &found), ("expected", &expected)]);
    let mut response = connect_error_with_status(&refusal, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    response.headers_mut().insert(
        HeaderName::from_static(ACCEPT_POST_HEADER),
        HeaderValue::from_static(accepted),
    );
    response
}

/// The Connect error envelope of a refusal: its Connect code, its message, and the diagnostic as the one detail.
pub fn connect_error_json(refusal: &Diagnostic) -> Value {
    let (code, _status) = connect_code(&refusal.code);
    let proto = diagnostic_to_proto(refusal);
    let debug: Value = serde_json::from_str(&diagnostic_to_json(&proto)).unwrap_or(Value::Null);
    json!({
        "code": code,
        "message": refusal.message,
        "details": [{
            "type": DIAGNOSTIC_TYPE_NAME,
            "value": base64_without_padding(&proto.encode_to_vec()),
            "debug": debug,
        }],
    })
}

/// One message of a server stream in its envelope.
pub fn stream_message_envelope(message: &[u8]) -> Vec<u8> {
    envelope(MESSAGE_FLAGS, message)
}

/// The last envelope of a server stream: `{}` when it ended well, else `{"error": <the error envelope>}`.
pub fn end_of_stream_envelope(error: Option<&Diagnostic>) -> Vec<u8> {
    let body = match error {
        Some(refusal) => json!({ "error": connect_error_json(refusal) }),
        None => json!({}),
    };
    envelope(END_OF_STREAM_FLAGS, body.to_string().as_bytes())
}

fn connect_error_with_status(refusal: &Diagnostic, status: StatusCode) -> Response {
    let body = connect_error_json(refusal).to_string();
    (status, [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)], body).into_response()
}

/// The Connect code and HTTP status for a catalog code. Anything that isn't a credential, a refused origin or scope,
/// or an unreadable body is the daemon's own failure.
fn connect_code(code: &str) -> (&'static str, StatusCode) {
    if UNAUTHENTICATED_CODES.contains(&code) {
        return (CONNECT_UNAUTHENTICATED, StatusCode::UNAUTHORIZED);
    }
    if PERMISSION_DENIED_CODES.contains(&code) {
        return (CONNECT_PERMISSION_DENIED, StatusCode::FORBIDDEN);
    }
    if code == INVALID_ARGUMENT_CODE || code.starts_with(FORMATS_CODE_PREFIX) {
        return (CONNECT_INVALID_ARGUMENT, StatusCode::BAD_REQUEST);
    }
    (CONNECT_INTERNAL, StatusCode::INTERNAL_SERVER_ERROR)
}

fn undecodable_body(message_name: &str) -> Diagnostic {
    let found = format!("a body that isn't a protobuf {message_name}");
    let expected = format!("{message_name} in protobuf binary, as {PROTO_CONTENT_TYPE}");
    diagnostic(INVALID_ARGUMENT_CODE, &[("found", &found), ("expected", &expected)])
}

fn undecodable_stream_body() -> Diagnostic {
    let found = format!("a body that isn't one enveloped protobuf {SUBSCRIBE_REQUEST_NAME}");
    let expected = format!("{SUBSCRIBE_REQUEST_NAME} in one uncompressed envelope, as {STREAM_PROTO_CONTENT_TYPE}");
    diagnostic(INVALID_ARGUMENT_CODE, &[("found", &found), ("expected", &expected)])
}

fn envelope(flags: u8, message: &[u8]) -> Vec<u8> {
    // A message never nears 4 GiB: requests are capped far below that and events are small. Were one ever larger,
    // the length is written as the most a u32 holds rather than wrapped, so a reader stops instead of misreading.
    let length = u32::try_from(message.len()).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(ENVELOPE_HEADER_BYTES + message.len());
    bytes.push(flags);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(message);
    bytes
}

/// Three bytes become four characters; a last group of one or two bytes becomes two or three, with no `=`.
fn base64_without_padding(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(BASE64_GROUP_BYTES) * 4);
    for group in bytes.chunks(BASE64_GROUP_BYTES) {
        let mut bits: u32 = 0;
        for (index, byte) in group.iter().enumerate() {
            bits |= u32::from(*byte) << (16 - 8 * index);
        }
        let character_count = group.len() + 1;
        for index in 0..character_count {
            let six_bits = (bits >> (18 - 6 * index)) & BASE64_SIX_BITS;
            text.push(char::from(BASE64_ALPHABET[six_bits as usize]));
        }
    }
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn content_types_pick_the_codec() {
        assert_eq!(codec_for_content_type("application/proto"), Some(ConnectCodec::Proto));
        assert_eq!(codec_for_content_type("application/json"), Some(ConnectCodec::Json));
        assert_eq!(
            codec_for_content_type("Application/JSON; charset=utf-8"),
            Some(ConnectCodec::Json)
        );
        for refused in [
            "",
            "text/plain",
            "application/connect+proto",
            "application/jsonx",
            "application/grpc",
        ] {
            assert_eq!(codec_for_content_type(refused), None, "{refused}");
        }
    }

    #[test]
    fn base64_matches_rfc_4648_without_padding() {
        // RFC 4648 section 10's test vectors, with the `=` taken off.
        let vectors = [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in vectors {
            assert_eq!(base64_without_padding(input.as_bytes()), expected, "{input}");
        }
        assert_eq!(base64_without_padding(&[0xfb, 0xff]), "+/8");
    }

    #[test]
    fn refusals_get_their_connect_code_and_status() {
        assert_eq!(
            connect_code("RRS-C1002"),
            (CONNECT_UNAUTHENTICATED, StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            connect_code("RRS-C1006"),
            (CONNECT_UNAUTHENTICATED, StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            connect_code("RRS-C1003"),
            (CONNECT_PERMISSION_DENIED, StatusCode::FORBIDDEN)
        );
        assert_eq!(
            connect_code("RRS-C1004"),
            (CONNECT_PERMISSION_DENIED, StatusCode::FORBIDDEN)
        );
        assert_eq!(
            connect_code("RRS-C0002"),
            (CONNECT_INVALID_ARGUMENT, StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            connect_code("RRS-E1003"),
            (CONNECT_INVALID_ARGUMENT, StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            connect_code("RRS-S3005"),
            (CONNECT_INTERNAL, StatusCode::INTERNAL_SERVER_ERROR)
        );
    }

    #[test]
    fn a_subscribe_request_is_one_uncompressed_envelope() {
        let request = SubscribeRequest {
            after_bus_sequence: 7,
            event_types: vec!["run.metrics".to_string()],
            subject_ids: Vec::new(),
        };
        let body = stream_message_envelope(&request.encode_to_vec());
        assert_eq!(decode_subscribe_request(&body).unwrap(), request);
        // An empty message is a valid request: a new stream of every event.
        assert_eq!(
            decode_subscribe_request(&[0, 0, 0, 0, 0]).unwrap(),
            SubscribeRequest::default()
        );

        let mut compressed = body.clone();
        compressed[0] = 0b01;
        let mut trailing = body.clone();
        trailing.push(0);
        let refused = [
            Vec::new(),
            body[..4].to_vec(),
            body[..body.len() - 1].to_vec(),
            compressed,
            trailing,
        ];
        for bad in refused {
            assert_eq!(
                decode_subscribe_request(&bad).unwrap_err().code,
                INVALID_ARGUMENT_CODE,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn only_connect_proto_is_a_stream_content_type() {
        assert!(is_stream_content_type("application/connect+proto"));
        assert!(is_stream_content_type("Application/Connect+Proto; charset=x"));
        for refused in ["application/proto", "application/connect+json", "application/json", ""] {
            assert!(!is_stream_content_type(refused), "{refused}");
        }
    }

    #[test]
    fn an_envelope_is_flags_then_big_endian_length() {
        assert_eq!(stream_message_envelope(b"ab"), [0, 0, 0, 0, 2, b'a', b'b']);
        let end = end_of_stream_envelope(None);
        assert_eq!(end, [2, 0, 0, 0, 2, b'{', b'}']);
    }
}
