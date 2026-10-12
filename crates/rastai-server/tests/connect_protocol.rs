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
//! The Connect envelopes and content types join this file with the Execute endpoints (M1-27).

#![allow(clippy::unwrap_used)]
// A refusal is a `Diagnostic`; see connect_json.rs.
#![allow(clippy::result_large_err)]

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
