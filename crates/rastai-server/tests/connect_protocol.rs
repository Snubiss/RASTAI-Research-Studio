//! The Connect JSON of the `CommandService` messages against protobuf-es, the reference implementation (layout:
//! rust-control-plane.md 8.6 and 11.3). Two sets of fixtures, both checked against protobuf-es by
//! `packages/proto/src/golden.test.js`:
//!
//! - the golden messages, `fixtures/proto-golden/{commands,common}/<Name>.json` beside their `.binpb`: the JSON
//!   written for each `.binpb` is the same JSON value;
//! - `fixtures/proto-golden/commands/connect_json/cases.json`: the exact text protobuf-es writes for a message
//!   (`encode`), inputs it accepts with the text it writes back (`decode`), and inputs it refuses (`refuse`), with
//!   the code and path the daemon refuses them with.
//!
//! Then the Connect protocol through a running daemon (`end_to_end`): `Execute` and `ListCommands` in binary and JSON,
//! the content-type choice, the Connect error envelope for gate refusals and unreadable bodies (with the diagnostic as
//! its detail), a failed command as `ok: false` with HTTP 200, and the stream envelopes `Subscribe` writes (M1-28).

#![allow(clippy::unwrap_used)]
// A refusal is a `Diagnostic`; see connect_json.rs.
#![allow(clippy::result_large_err)]

mod common;

use prost::Message;
use rastai_diag::diagnostic::Diagnostic;
use rastai_proto::commands::v1::{
    CommandDescriptor, ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse,
};
use rastai_proto::common::v1::Diagnostic as ProtoDiagnostic;
use rastai_server::http::connect_json::{
    command_descriptor_from_json, command_descriptor_to_json, diagnostic_from_json, diagnostic_to_json,
    execute_request_from_json, execute_request_to_json, execute_response_from_json, execute_response_to_json,
    list_commands_request_from_json, list_commands_request_to_json, list_commands_response_from_json,
    list_commands_response_to_json,
};
use serde_json::Value;

const GOLDEN_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/proto-golden");
const CASES_FOLDER: &str = "commands/connect_json";
const CASES_FILE: &str = "cases.json";
/// The six messages the codec covers, each with the golden folder of its package.
const GOLDEN_MESSAGES: [(&str, &str); 6] = [
    ("commands", "ExecuteRequest"),
    ("commands", "ExecuteResponse"),
    ("common", "Diagnostic"),
    ("commands", "ListCommandsRequest"),
    ("commands", "ListCommandsResponse"),
    ("commands", "CommandDescriptor"),
];
/// A descriptor from a daemon that predates `stability` (fixtures/proto-golden/README.md).
const WITHOUT_STABILITY_FOLDER: &str = "commands/without_stability";

fn read_fixture(relative_path: &str) -> Vec<u8> {
    let path = format!("{GOLDEN_ROOT}/{relative_path}");
    std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn read_cases() -> Value {
    serde_json::from_slice(&read_fixture(&format!("{CASES_FOLDER}/{CASES_FILE}"))).unwrap()
}

/// The cases of one kind (`encode`, `decode` or `refuse`).
fn cases_of(cases: &Value, kind: &str) -> Vec<Value> {
    cases[kind].as_array().unwrap().clone()
}

fn text_of(case: &Value, key: &str) -> String {
    case[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} in {case}"))
        .to_string()
}

/// The binary message, decoded by prost, written as Connect JSON. This side never touches the JSON decoder.
fn json_from_binary(message_name: &str, binary: &[u8]) -> String {
    match message_name {
        "ExecuteRequest" => execute_request_to_json(&ExecuteRequest::decode(binary).unwrap()),
        "ExecuteResponse" => execute_response_to_json(&ExecuteResponse::decode(binary).unwrap()),
        "Diagnostic" => diagnostic_to_json(&ProtoDiagnostic::decode(binary).unwrap()),
        "ListCommandsRequest" => list_commands_request_to_json(&ListCommandsRequest::decode(binary).unwrap()),
        "ListCommandsResponse" => list_commands_response_to_json(&ListCommandsResponse::decode(binary).unwrap()),
        "CommandDescriptor" => command_descriptor_to_json(&CommandDescriptor::decode(binary).unwrap()),
        other => panic!("{other} isn't one of the six Connect JSON messages"),
    }
}

/// True when the JSON decodes to the same message prost decodes from the binary.
fn json_decodes_to_binary(message_name: &str, json: &[u8], binary: &[u8]) -> bool {
    match message_name {
        "ExecuteRequest" => execute_request_from_json(json).unwrap() == ExecuteRequest::decode(binary).unwrap(),
        "ExecuteResponse" => execute_response_from_json(json).unwrap() == ExecuteResponse::decode(binary).unwrap(),
        "Diagnostic" => diagnostic_from_json(json).unwrap() == ProtoDiagnostic::decode(binary).unwrap(),
        "ListCommandsRequest" => {
            list_commands_request_from_json(json).unwrap() == ListCommandsRequest::decode(binary).unwrap()
        }
        "ListCommandsResponse" => {
            list_commands_response_from_json(json).unwrap() == ListCommandsResponse::decode(binary).unwrap()
        }
        "CommandDescriptor" => {
            command_descriptor_from_json(json).unwrap() == CommandDescriptor::decode(binary).unwrap()
        }
        other => panic!("{other} isn't one of the six Connect JSON messages"),
    }
}

/// Decodes the input and writes the message back, or the refusal.
fn decode_and_write_back(message_name: &str, input: &[u8]) -> Result<String, Diagnostic> {
    match message_name {
        "ExecuteRequest" => execute_request_from_json(input).map(|message| execute_request_to_json(&message)),
        "ExecuteResponse" => execute_response_from_json(input).map(|message| execute_response_to_json(&message)),
        "Diagnostic" => diagnostic_from_json(input).map(|message| diagnostic_to_json(&message)),
        "ListCommandsRequest" => {
            list_commands_request_from_json(input).map(|message| list_commands_request_to_json(&message))
        }
        "ListCommandsResponse" => {
            list_commands_response_from_json(input).map(|message| list_commands_response_to_json(&message))
        }
        "CommandDescriptor" => command_descriptor_from_json(input).map(|message| command_descriptor_to_json(&message)),
        other => panic!("{other} isn't one of the six Connect JSON messages"),
    }
}

#[test]
fn each_golden_message_is_written_as_its_golden_json() {
    for (folder, message_name) in GOLDEN_MESSAGES {
        let binary = read_fixture(&format!("{folder}/{message_name}.binpb"));
        let golden: Value = serde_json::from_slice(&read_fixture(&format!("{folder}/{message_name}.json"))).unwrap();
        let written: Value = serde_json::from_str(&json_from_binary(message_name, &binary)).unwrap();
        assert_eq!(written, golden, "{message_name}");
    }
}

#[test]
fn a_descriptor_without_stability_is_written_without_it() {
    let binary = read_fixture(&format!("{WITHOUT_STABILITY_FOLDER}/CommandDescriptor.binpb"));
    let golden: Value = serde_json::from_slice(&read_fixture(&format!(
        "{WITHOUT_STABILITY_FOLDER}/CommandDescriptor.json"
    )))
    .unwrap();
    let written: Value = serde_json::from_str(&json_from_binary("CommandDescriptor", &binary)).unwrap();
    assert_eq!(written, golden);
    assert!(written.get("stability").is_none());
}

#[test]
fn each_golden_json_decodes_to_its_binary_message() {
    for (folder, message_name) in GOLDEN_MESSAGES {
        let binary = read_fixture(&format!("{folder}/{message_name}.binpb"));
        let json = read_fixture(&format!("{folder}/{message_name}.json"));
        assert!(json_decodes_to_binary(message_name, &json, &binary), "{message_name}");
    }
}

#[test]
fn every_message_is_written_byte_for_byte_as_protobuf_es_writes_it() {
    let cases = read_cases();
    let encode_cases = cases_of(&cases, "encode");
    for (_, message_name) in GOLDEN_MESSAGES {
        let covered = encode_cases.iter().any(|case| text_of(case, "message") == message_name);
        assert!(covered, "no encode case for {message_name}");
    }
    for case in encode_cases {
        let message_name = text_of(&case, "message");
        let binary = read_fixture(&format!("{CASES_FOLDER}/{}", text_of(&case, "binary")));
        let expected = text_of(&case, "json");
        assert_eq!(json_from_binary(&message_name, &binary), expected, "{case}");
        assert!(
            json_decodes_to_binary(&message_name, expected.as_bytes(), &binary),
            "{case}"
        );
    }
}

#[test]
fn inputs_protobuf_es_accepts_decode_to_what_it_writes_back() {
    let cases = read_cases();
    for case in cases_of(&cases, "decode") {
        let message_name = text_of(&case, "message");
        let input = text_of(&case, "input");
        let written = decode_and_write_back(&message_name, input.as_bytes());
        assert_eq!(
            written.map_err(|refusal| refusal.code),
            Ok(text_of(&case, "json")),
            "{case}"
        );
    }
}

#[test]
fn inputs_protobuf_es_refuses_are_refused_with_a_code_and_a_path() {
    let cases = read_cases();
    for case in cases_of(&cases, "refuse") {
        let message_name = text_of(&case, "message");
        let input = text_of(&case, "input");
        let refusal = decode_and_write_back(&message_name, input.as_bytes()).unwrap_err();
        let expected = (text_of(&case, "code"), text_of(&case, "path"));
        assert_eq!((refusal.code, refusal.path), expected, "{case}");
    }
}

/// The same messages through a running daemon's `CommandService` endpoints.
#[cfg(unix)]
mod end_to_end {
    use prost::Message;
    use rastai_core::commands::table::COMMANDS;
    use rastai_diag::diagnostic::diagnostic;
    use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse};
    use rastai_proto::common::v1::Diagnostic as ProtoDiagnostic;
    use rastai_server::http::connect::{end_of_stream_envelope, stream_message_envelope};
    use rastai_server::http::connect_json::{execute_response_from_json, list_commands_response_from_json};
    use rastai_transport::http_client::HttpResponse;
    use serde_json::Value;

    use crate::common::{
        header_value, owner_bearer, refusal_code, remove_test_folder, send_to_port, start_quiet_test_daemon,
        start_test_daemon, stop_test_daemon, test_paths,
    };

    const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
    const LIST_COMMANDS_PATH: &str = "/rastai.commands.v1.CommandService/ListCommands";
    const PROTO: &str = "application/proto";
    const JSON: &str = "application/json";
    const BASE64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    async fn post(port: u16, path: &str, bearer: &str, content_type: &str, body: &[u8]) -> HttpResponse {
        let headers = [("Authorization", bearer), ("Content-Type", content_type)];
        send_to_port(port, "POST", path, &headers, body).await
    }

    fn json_body(response: &HttpResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    /// A test-side reader of the standard base64 Connect writes details in, padded or not.
    fn decode_base64(text: &str) -> Vec<u8> {
        let mut bits: u32 = 0;
        let mut bit_count = 0;
        let mut bytes = Vec::new();
        for character in text.bytes().filter(|byte| *byte != b'=') {
            let value = BASE64_ALPHABET.iter().position(|known| *known == character).unwrap() as u32;
            bits = (bits << 6) | value;
            bit_count += 6;
            if bit_count >= 8 {
                bit_count -= 8;
                bytes.push((bits >> bit_count) as u8);
                bits &= (1 << bit_count) - 1;
            }
        }
        bytes
    }

    /// The diagnostic a Connect error envelope carries, read from its binary `value`, and its Connect code.
    fn connect_error_of(response: &HttpResponse) -> (String, ProtoDiagnostic) {
        assert_eq!(header_value(response, "content-type"), Some(JSON));
        let body = json_body(response);
        let detail = &body["details"][0];
        assert_eq!(detail["type"], "rastai.common.v1.Diagnostic");
        let diagnostic = ProtoDiagnostic::decode(decode_base64(detail["value"].as_str().unwrap()).as_slice()).unwrap();
        assert_eq!(detail["debug"]["code"], diagnostic.code.as_str());
        assert_eq!(body["message"], diagnostic.message.as_str());
        (body["code"].as_str().unwrap().to_string(), diagnostic)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_answers_in_the_codec_it_was_asked_in() {
        let paths = test_paths("cp-execute");
        // A quiet node: system.status names its last heartbeat, which must not move between the two answers.
        let running = start_quiet_test_daemon(&paths, &[]).await;
        let bearer = owner_bearer(&paths);
        let request = ExecuteRequest {
            command_id: "system.status".to_string(),
            ..ExecuteRequest::default()
        };
        let binary = post(
            running.http_port,
            EXECUTE_PATH,
            &bearer,
            PROTO,
            &request.encode_to_vec(),
        )
        .await;
        assert_eq!(binary.status, 200);
        assert_eq!(header_value(&binary, "content-type"), Some(PROTO));
        let answer = ExecuteResponse::decode(binary.body.as_slice()).unwrap();
        assert!(answer.ok, "{:?}", answer.diagnostics);
        assert!(answer.result_json.contains("rastai.commands.v1"));

        // connect-es sends the lowerCamelCase names; the codec reads both spellings.
        let json_request = br#"{"commandId": "system.status", "argumentsJson": "{}"}"#;
        let json_answer = post(
            running.http_port,
            EXECUTE_PATH,
            &bearer,
            "application/json; charset=utf-8",
            json_request,
        )
        .await;
        assert_eq!(json_answer.status, 200);
        assert_eq!(header_value(&json_answer, "content-type"), Some(JSON));
        let decoded = execute_response_from_json(&json_answer.body).unwrap();
        assert_eq!(decoded.result_json, answer.result_json);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_command_is_ok_false_with_http_200() {
        let paths = test_paths("cp-failed");
        let running = start_test_daemon(&paths).await;
        let bearer = owner_bearer(&paths);
        let response = post(
            running.http_port,
            EXECUTE_PATH,
            &bearer,
            JSON,
            br#"{"command_id": "no.such"}"#,
        )
        .await;
        assert_eq!(response.status, 200);
        let decoded = execute_response_from_json(&response.body).unwrap();
        assert!(!decoded.ok);
        assert_eq!(decoded.diagnostics[0].code, "RRS-C0001");
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_commands_lists_the_registry_with_schemas() {
        let paths = test_paths("cp-list");
        let running = start_test_daemon(&paths).await;
        let bearer = owner_bearer(&paths);
        let binary = post(
            running.http_port,
            LIST_COMMANDS_PATH,
            &bearer,
            PROTO,
            &ListCommandsRequest::default().encode_to_vec(),
        )
        .await;
        assert_eq!(binary.status, 200);
        let listed = ListCommandsResponse::decode(binary.body.as_slice()).unwrap();
        assert_eq!(listed.commands.len(), COMMANDS.len());
        for command in &listed.commands {
            assert!(!command.arguments_schema_json.is_empty(), "{}", command.command_id);
            assert!(!command.result_type.is_empty(), "{}", command.command_id);
        }
        let section = post(
            running.http_port,
            LIST_COMMANDS_PATH,
            &bearer,
            JSON,
            br#"{"section": "remote"}"#,
        )
        .await;
        let ids: Vec<String> = list_commands_response_from_json(&section.body)
            .unwrap()
            .commands
            .into_iter()
            .map(|command| command.command_id)
            .collect();
        assert_eq!(ids, ["node.list", "node.status"]);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gate_refusals_come_in_the_connect_error_envelope() {
        let paths = test_paths("cp-gate");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let no_credential = send_to_port(port, "POST", EXECUTE_PATH, &[("Content-Type", JSON)], b"{}").await;
        assert_eq!(no_credential.status, 401);
        let (code, diagnostic) = connect_error_of(&no_credential);
        assert_eq!(
            (code.as_str(), diagnostic.code.as_str()),
            ("unauthenticated", "RRS-C1002")
        );
        let rebinding = send_to_port(port, "POST", LIST_COMMANDS_PATH, &[("Host", "attacker.test")], b"{}").await;
        assert_eq!(rebinding.status, 403);
        let (code, diagnostic) = connect_error_of(&rebinding);
        assert_eq!(
            (code.as_str(), diagnostic.code.as_str()),
            ("permission_denied", "RRS-C1003")
        );
        assert_eq!(refusal_code(&rebinding), "RRS-C1003");
        // Off the Connect paths a refusal stays the plain diagnostic.
        let login = send_to_port(port, "POST", "/api/login", &[("Host", "attacker.test")], b"{}").await;
        assert_eq!(json_body(&login)["code"], "RRS-C1003");
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bodies_that_cannot_be_read_are_refused_before_any_command() {
        let paths = test_paths("cp-body");
        let running = start_test_daemon(&paths).await;
        let bearer = owner_bearer(&paths);
        let port = running.http_port;
        let plain_text = post(port, EXECUTE_PATH, &bearer, "text/plain", b"system.status").await;
        assert_eq!(plain_text.status, 415);
        assert_eq!(
            header_value(&plain_text, "accept-post"),
            Some("application/proto, application/json")
        );
        assert_eq!(connect_error_of(&plain_text).1.code, "RRS-C0002");
        let not_proto = post(port, EXECUTE_PATH, &bearer, PROTO, &[0xff, 0xff, 0xff]).await;
        assert_eq!(not_proto.status, 400);
        let (code, diagnostic) = connect_error_of(&not_proto);
        assert_eq!(
            (code.as_str(), diagnostic.code.as_str()),
            ("invalid_argument", "RRS-C0002")
        );
        let unknown_key = post(port, EXECUTE_PATH, &bearer, JSON, br#"{"command": "system.status"}"#).await;
        assert_eq!(unknown_key.status, 400);
        let (_, diagnostic) = connect_error_of(&unknown_key);
        assert_eq!(
            (diagnostic.code.as_str(), diagnostic.path.as_str()),
            ("RRS-E1003", "#/command")
        );
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[test]
    fn stream_envelopes_carry_flags_a_length_and_the_end_of_stream_error() {
        assert_eq!(stream_message_envelope(&[7, 8, 9]), [0, 0, 0, 0, 3, 7, 8, 9]);
        let refusal = diagnostic("RRS-C1006", &[]);
        let end = end_of_stream_envelope(Some(&refusal));
        assert_eq!(end[0], 2);
        let length = u32::from_be_bytes([end[1], end[2], end[3], end[4]]) as usize;
        assert_eq!(length, end.len() - 5);
        let trailer: Value = serde_json::from_slice(&end[5..]).unwrap();
        assert_eq!(trailer["error"]["code"], "unauthenticated");
        assert_eq!(trailer["error"]["details"][0]["debug"]["code"], "RRS-C1006");
    }
}
