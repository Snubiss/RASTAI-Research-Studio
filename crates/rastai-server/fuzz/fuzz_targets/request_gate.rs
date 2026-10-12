//! Fuzz target for the request gate's header reading (contracts 12.5): a request head on the daemon's port comes
//! from any process or web page that reached it, so arbitrary bytes must never panic `read_request_head` or
//! `check_request_gate`, and a malformed head must be refused, never let through:
//!
//! - the input is read as a request head: a first line `METHOD PATH`, then `name: value` lines;
//! - whatever `read_request_head` accepts goes through the gate over loopback TCP, with one live session, an owner
//!   token, a scoped token and a revoked one planted under short secrets, so the cookie, key and every bearer
//!   branch are reachable too;
//! - a request the gate lets through has a Host that is a loopback name, and, when it carries an Origin, that
//!   Origin is `http://` plus the Host.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_core::scopes::Scope;
use rastai_formats::hashing::hash_bytes;
use rastai_server::http::gate::{Listener, check_request_gate, read_request_head};
use rastai_server::http::sessions::{AuthState, ScopedToken, Session, add_scoped_token, new_auth_state};

const COOKIE_NAME: &str = "rastai_session_0a1b2c3d";
/// Short secrets, so the fuzzer can find them: `Cookie: rastai_session_0a1b2c3d=s`, `Rastai-Session-Key: k`,
/// `Authorization: Bearer t`, and the scoped tokens `Bearer u` (live) and `Bearer v` (revoked).
const SESSION_ID: &[u8] = b"s";
const SESSION_KEY: &[u8] = b"k";
const OWNER_TOKEN: &[u8] = b"t";
const SCOPED_TOKEN: &[u8] = b"u";
const REVOKED_TOKEN: &[u8] = b"v";
const SESSION_ORIGIN: &str = "http://127.0.0.1:47820";
/// Far in the future, so the planted session never goes idle during a run.
const LAST_USED_UNIX_MS: i64 = i64::MAX / 2;
const LOOPBACK_NAMES: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

fuzz_target!(|bytes: &[u8]| {
    let auth = planted_auth();
    check_head(bytes, &auth);
});

fn planted_auth() -> AuthState {
    let auth = new_auth_state(hash_bytes(OWNER_TOKEN), "0a1b2c3d");
    if let Ok(mut sessions) = auth.sessions.lock() {
        sessions.push(Session {
            session_id_hash: hash_bytes(SESSION_ID),
            bound_origin: SESSION_ORIGIN.to_string(),
            key_hash: hash_bytes(SESSION_KEY),
            last_used_unix_ms: LAST_USED_UNIX_MS,
        });
    }
    for (token_id, secret, revoked) in [("tok_u", SCOPED_TOKEN, false), ("tok_v", REVOKED_TOKEN, true)] {
        let token = ScopedToken {
            token_id: token_id.to_string(),
            token_hash: hash_bytes(secret),
            scopes: vec![Scope::Read],
            revoked,
        };
        add_scoped_token(&auth, token);
    }
    auth
}

fn check_head(bytes: &[u8], auth: &AuthState) {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let first_line = String::from_utf8_lossy(lines.next().unwrap_or_default()).into_owned();
    let (method, path) = first_line.split_once(' ').unwrap_or((first_line.as_str(), "/"));
    let headers = header_lines(lines);
    let header_refs: Vec<(&str, &[u8])> = headers.iter().map(|(name, value)| (name.as_str(), value.as_slice())).collect();
    let Ok(head) = read_request_head(method, path, &header_refs, COOKIE_NAME) else {
        return;
    };
    if check_request_gate(&head, &Listener::LoopbackTcp, auth).is_err() {
        return;
    }
    assert!(LOOPBACK_NAMES.contains(&host_name_of(&head.host)), "passed with Host {:?}", head.host);
    if let Some(origin) = &head.origin {
        assert_eq!(*origin, format!("http://{}", head.host), "passed with a foreign Origin");
    }
}

/// The name part of a Host, read independently of the gate: `[::1]` up to its bracket, else up to the first colon.
fn host_name_of(host: &str) -> &str {
    if let Some(end) = host.find(']') {
        return &host[..=end];
    }
    host.split(':').next().unwrap_or(host)
}

/// `name: value` lines; a line without a colon is a header with an empty value, as a sloppy client might send.
fn header_lines<'a>(lines: impl Iterator<Item = &'a [u8]>) -> Vec<(String, Vec<u8>)> {
    let mut headers = Vec::new();
    for line in lines {
        let colon = line.iter().position(|byte| *byte == b':').unwrap_or(line.len());
        let name = String::from_utf8_lossy(&line[..colon]).trim().to_lowercase();
        let value = line.get(colon + 1..).unwrap_or_default();
        headers.push((name, value.to_vec()));
    }
    headers
}
