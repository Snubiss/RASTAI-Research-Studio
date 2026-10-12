//! Fuzz target for the hand-written Connect JSON decoders (contracts 12.5, ADR-0037): a Connect JSON body comes
//! from any process or web page that passed the gate, so arbitrary bytes must never panic a decoder:
//!
//! - the first byte picks one of the six messages, and the rest is its body;
//! - a refusal carries one of the codes the decoder documents: the strict reader's, or a field's;
//! - a body the decoder accepts writes back as JSON that decodes to the same message and writes back to the same
//!   text, so writing and reading agree on every message the fuzzer finds.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_diag::diagnostic::Diagnostic;
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

fuzz_target!(|bytes: &[u8]| {
    let Some((first_byte, body)) = bytes.split_first() else {
        return;
    };
    match first_byte % MESSAGE_COUNT {
        0 => check(body, execute_request_from_json, execute_request_to_json),
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
