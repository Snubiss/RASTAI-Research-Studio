//! Fuzz target for the hand-written Connect JSON decoders (contracts 12.5, ADR-0037): a Connect JSON body comes
//! from any process or web page that passed the gate, so arbitrary bytes must never panic a decoder:
//!
//! - the first byte picks one of the six messages, and the rest is its body;
//! - a refusal carries one of the codes the decoder documents: the strict reader's, or a field's;
//! - a body the decoder accepts writes back as JSON that decodes to the same message and writes back to the same
//!   text, so writing and reading agree on every message the fuzzer finds;
//! - an `ExecuteRequest`'s `arguments_json` then goes through `check_arguments` for every core command, as the
//!   daemon does before any handler runs: it passes, or it is refused with `RRS-C0002` and `RRS-E` problems.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::commands::COMMAND_TYPES;
use rastai_proto::commands::v1::{CommandDescriptor, ExecuteRequest};
use rastai_server::commands::arguments::check_arguments;
use rastai_server::http::connect_json::{
    command_descriptor_from_json, command_descriptor_to_json, diagnostic_from_json, diagnostic_to_json,
    execute_request_from_json, execute_request_to_json, execute_response_from_json, execute_response_to_json,
    list_commands_request_from_json, list_commands_request_to_json, list_commands_response_from_json,
    list_commands_response_to_json,
};

const MESSAGE_COUNT: u8 = 6;
/// What connect_json.rs refuses with: the strict reader's syntax and limit codes, then the field codes.
const REFUSAL_CODES: [&str; 10] = [
    "RRS-E0001",
    "RRS-E0002",
    "RRS-E0003",
    "RRS-E0004",
    "RRS-E5003",
    "RRS-E5004",
    "RRS-E5006",
    "RRS-E1002",
    "RRS-E1003",
    "RRS-E1004",
];
/// What leads every refusal of `check_arguments`; the problems after it are `RRS-E` codes.
const INVALID_ARGUMENTS_CODE: &str = "RRS-C0002";
const FORMATS_CODE_PREFIX: &str = "RRS-E";

fuzz_target!(|bytes: &[u8]| {
    let Some((first_byte, body)) = bytes.split_first() else {
        return;
    };
    match first_byte % MESSAGE_COUNT {
        0 => {
            check(body, execute_request_from_json, execute_request_to_json);
            if let Ok(request) = execute_request_from_json(body) {
                check_every_command_arguments(&request);
            }
        }
        1 => check(body, execute_response_from_json, execute_response_to_json),
        2 => check(body, diagnostic_from_json, diagnostic_to_json),
        3 => check(body, list_commands_request_from_json, list_commands_request_to_json),
        4 => check(body, list_commands_response_from_json, list_commands_response_to_json),
        _ => check(body, command_descriptor_from_json, command_descriptor_to_json),
    }
});

/// Decodes the body; a refusal must carry a documented code, and an accepted message must survive writing and
/// reading back unchanged.
fn check<M: PartialEq + std::fmt::Debug>(
    body: &[u8],
    decode: fn(&[u8]) -> Result<M, Diagnostic>,
    encode: fn(&M) -> String,
) {
    let message = match decode(body) {
        Ok(message) => message,
        Err(refusal) => {
            assert!(
                REFUSAL_CODES.contains(&refusal.code.as_str()),
                "refused with {}",
                refusal.code
            );
            return;
        }
    };
    let written = encode(&message);
    let read_back = match decode(written.as_bytes()) {
        Ok(read_back) => read_back,
        Err(refusal) => panic!("its own output {written} was refused with {}", refusal.code),
    };
    assert_eq!(read_back, message, "{written}");
    assert_eq!(encode(&read_back), written);
}

/// The request's arguments against every core command's description, and against a plugin's (no description).
fn check_every_command_arguments(request: &ExecuteRequest) {
    let mut command_ids: Vec<&str> = COMMAND_TYPES.iter().map(|row| row.command_id).collect();
    command_ids.push("acme.tool.run");
    for command_id in command_ids {
        let command = CommandDescriptor {
            command_id: command_id.to_string(),
            ..CommandDescriptor::default()
        };
        let Err(refusal) = check_arguments(&command, &request.arguments_json) else {
            continue;
        };
        assert_eq!(refusal[0].code, INVALID_ARGUMENTS_CODE, "{command_id}");
        assert!(refusal.len() > 1, "{command_id}: no problem after RRS-C0002");
        for problem in &refusal[1..] {
            assert!(problem.code.starts_with(FORMATS_CODE_PREFIX), "{command_id}: {}", problem.code);
        }
    }
}
