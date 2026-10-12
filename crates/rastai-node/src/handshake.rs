//! `NodeService.Handshake`, the node's side (layout: rust-node-gateway-proto.md section 7.9; designer flow C3):
//! the protocol version, the capability flags, the daemon's signed challenge checked against the pinned daemon keys,
//! and the node's own signature over the session token it hands out.
//!
//! The checks run cheapest and least revealing first: the version (an older daemon must always learn ours, so it can
//! offer node.update), then the pinned key and the signature, and only then freshness, so a caller without a paired
//! key can't fill the node's memory of used nonces.
//!
//! Signed bytes are the domain string and the fields in a fixed order, each text or byte field preceded by its length
//! as 4 big-endian bytes and the time as 8 big-endian bytes, so no two field lists give the same bytes. The domain
//! string stops a signature from being reused anywhere else. The daemon signs
//! `(domain, node_id, daemon_nonce, sent_unix_ms, hub_socket_path)`; the node signs
//! `(domain, daemon_id, daemon_nonce, BLAKE3 of session_token)`.
//!
//! `<data>/node/paired_daemons.json` is `{"format_version": 1, "daemon_ids": ["daemon_<base32>", ...]}`. The id is
//! the daemon's public key, so the file pins keys without a second field. The daemon's first start pins its own key
//! for the embedded node, and `rastai node init --daemon-key` pins it on a box (M9).

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use rastai_core::crockford::encode_base32;
use rastai_core::paths::paired_daemons_path;
use rastai_core::random::random_bytes;
use rastai_core::time::now_unix_ms;
use rastai_core::versions::{APP_VERSION, NODE_PROTOCOL_VERSION};
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::logger::log_diagnostic;
use rastai_formats::hashing::hash_bytes;
use rastai_proto::node::v1::{HandshakeRequest, HandshakeResponse};
use serde_json::{Value, json};

use crate::error::{NodeError, node_error_diagnostic};
use crate::identity::{
    public_key_from_id, sign_challenge, temporary_path_beside, verify_challenge, write_private_file,
};
use crate::node::{NodeState, current_hardware};

/// The first field of every signed handshake message.
const HANDSHAKE_DOMAIN: &str = "rastai-node-handshake-v1";
/// The daemon's clock may differ from ours by this much (the proto comment on `sent_unix_ms`). A recorded handshake
/// older than this is refused even before its nonce would be forgotten.
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;
/// A nonce shorter than this can't be trusted to be fresh; the daemon sends 32 random bytes.
const MIN_NONCE_BYTES: usize = 16;
/// The session token: 32 random bytes as base32 text, like the job token (contracts 3.2).
const SESSION_TOKEN_BYTES: usize = 32;
/// Used nonces are kept this long: past it, the clock check refuses a replay on its own.
const NONCE_MEMORY_MS: u64 = 2 * MAX_CLOCK_SKEW_MS;
/// At most this many used nonces are kept. A daemon handshakes once per connection, so a full list within ten minutes
/// means only signed requests from a paired daemon arrived that fast; the oldest is then forgotten.
const NONCE_MEMORY_LIMIT: usize = 4096;
const DAEMON_ID_PREFIX: &str = "daemon_";
const PAIRED_DAEMONS_FORMAT_VERSION: u64 = 1;
const FORMAT_VERSION_KEY: &str = "format_version";
const DAEMON_IDS_KEY: &str = "daemon_ids";
/// paired_daemons.json decides who may drive this node, so nobody but its owner may write it. It is written mode
/// 0600 (identity.rs's private files).
#[cfg(unix)]
const GROUP_AND_OTHER_WRITE_BITS: u32 = 0o022;

/// The daemon that holds the current session. A reconnect replaces it (designer flow C3a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonSession {
    pub daemon_id: String,
    /// Sent in gRPC metadata on later calls both ways; in memory only (designer flow C3a).
    pub session_token: String,
    /// The node-hub socket the hub client (M1-45) dials for NodeHubService.
    pub hub_socket_path: String,
}

/// The handshake's in-memory state: the current session and the nonces seen lately.
#[derive(Debug, Default)]
pub struct HandshakeTable {
    pub session: Option<DaemonSession>,
    recent_nonces: Vec<(Vec<u8>, i64)>,
}

/// The node's answer to a Handshake: accepted with a session token and the node's signature, or refused with a
/// diagnostic. Either way it names the node and its versions, which the daemon needs to offer node.update.
pub fn answer_handshake(state: &NodeState, request: &HandshakeRequest) -> HandshakeResponse {
    let mut response = HandshakeResponse {
        node_id: state.node_id.clone(),
        protocol_version: NODE_PROTOCOL_VERSION,
        app_version: APP_VERSION.to_string(),
        capabilities: node_capabilities(),
        ..HandshakeResponse::default()
    };
    match accept_handshake(state, request, now_unix_ms()) {
        Ok(session_token) => {
            let signed = node_challenge_bytes(&request.daemon_id, &request.daemon_nonce, &session_token);
            response.node_signature = sign_challenge(&state.identity, &signed);
            response.hardware = Some(current_hardware(state));
            response.session_token = session_token;
        }
        Err(error) => {
            let refusal = node_error_diagnostic(&error);
            log_diagnostic(&refusal, &[("daemon_id", &request.daemon_id)]);
            response.diagnostic = Some(diagnostic_to_proto(&refusal));
        }
    }
    response
}

/// The bytes the daemon signs with daemon.key and the node verifies.
pub fn daemon_challenge_bytes(node_id: &str, daemon_nonce: &[u8], sent_unix_ms: i64, hub_socket_path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_field(&mut bytes, HANDSHAKE_DOMAIN.as_bytes());
    push_field(&mut bytes, node_id.as_bytes());
    push_field(&mut bytes, daemon_nonce);
    bytes.extend_from_slice(&sent_unix_ms.to_be_bytes());
    push_field(&mut bytes, hub_socket_path.as_bytes());
    bytes
}

/// The bytes the node signs with node.key and the daemon verifies against the key it pinned. The token itself is
/// never signed in the clear: its BLAKE3 hash stands for it.
pub fn node_challenge_bytes(daemon_id: &str, daemon_nonce: &[u8], session_token: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_field(&mut bytes, HANDSHAKE_DOMAIN.as_bytes());
    push_field(&mut bytes, daemon_id.as_bytes());
    push_field(&mut bytes, daemon_nonce);
    push_field(&mut bytes, hash_bytes(session_token.as_bytes()).as_bytes());
    bytes
}

/// The daemon ids pinned in paired_daemons.json. No file means no daemon is paired yet.
pub fn read_paired_daemons(path: &Path) -> Result<Vec<String>, NodeError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(unusable(path, format!("can't be read: {error}"))),
    };
    check_owner_only_writes(path)?;
    parse_paired_daemons(&text).map_err(|reason| unusable(path, reason))
}

/// Adds `daemon_id` to paired_daemons.json, which is written whole to a temporary file of this call's own, mode 0600,
/// synced, and renamed over the old one. Pinning a daemon that is already pinned changes nothing.
pub fn pin_daemon_key(path: &Path, daemon_id: &str) -> Result<(), NodeError> {
    if public_key_from_id(daemon_id, DAEMON_ID_PREFIX).is_none() {
        return Err(unusable(path, format!("{daemon_id} isn't a daemon key id")));
    }
    let mut daemon_ids = read_paired_daemons(path)?;
    if daemon_ids.iter().any(|pinned| pinned == daemon_id) {
        return Ok(());
    }
    daemon_ids.push(daemon_id.to_string());
    write_paired_daemons(path, &daemon_ids)
}

/// The capability flags of contracts 8.6 this node has. None is built yet: `generate.inspectable` and
/// `generate.fast` join with M3, `lens.live` with M4, `sandbox.engine` with M5, `gym` with M7, `transfer.resume`
/// and `self_update` with M9. The scheduler refuses a job whose flag is missing (RRS-N0004).
fn node_capabilities() -> Vec<String> {
    Vec::new()
}

/// Every check, in order; on success the new session's token, which replaces any earlier session.
fn accept_handshake(state: &NodeState, request: &HandshakeRequest, now_ms: i64) -> Result<String, NodeError> {
    check_protocol_version(state, request)?;
    check_daemon_signature(state, request)?;
    remember_fresh_nonce(state, request, now_ms)?;
    let session_token = encode_base32(&random_bytes(SESSION_TOKEN_BYTES)?);
    let session = DaemonSession {
        daemon_id: request.daemon_id.clone(),
        session_token: session_token.clone(),
        hub_socket_path: request.hub_socket_path.clone(),
    };
    lock_handshakes(state).session = Some(session);
    Ok(session_token)
}

fn check_protocol_version(state: &NodeState, request: &HandshakeRequest) -> Result<(), NodeError> {
    if request.protocol_version == NODE_PROTOCOL_VERSION {
        return Ok(());
    }
    Err(NodeError::ProtocolVersionUnsupported {
        node_id: state.node_id.clone(),
        node_version: NODE_PROTOCOL_VERSION,
        daemon_version: request.protocol_version,
    })
}

/// The daemon id must be pinned, must be a key, and its key must have signed this request for this node.
fn check_daemon_signature(state: &NodeState, request: &HandshakeRequest) -> Result<(), NodeError> {
    let not_paired = |reason: String| NodeError::DaemonNotPaired {
        node_id: state.node_id.clone(),
        reason,
    };
    let pinned = read_paired_daemons(&paired_daemons_path(&state.paths))?;
    if !pinned.iter().any(|daemon_id| daemon_id == &request.daemon_id) {
        return Err(not_paired(format!("{} isn't pinned", request.daemon_id)));
    }
    let Some(public_key) = public_key_from_id(&request.daemon_id, DAEMON_ID_PREFIX) else {
        return Err(not_paired(format!("{} isn't a daemon key id", request.daemon_id)));
    };
    let signed = daemon_challenge_bytes(
        &state.node_id,
        &request.daemon_nonce,
        request.sent_unix_ms,
        &request.hub_socket_path,
    );
    verify_challenge(&public_key, &signed, &request.daemon_signature).map_err(|error| match error {
        NodeError::SignatureRefused { reason } => not_paired(reason),
        other => other,
    })
}

/// Refuses a request whose clock is too far from ours or whose nonce was seen before, then remembers its nonce.
fn remember_fresh_nonce(state: &NodeState, request: &HandshakeRequest, now_ms: i64) -> Result<(), NodeError> {
    let stale = |reason: String| NodeError::HandshakeStale {
        daemon_id: request.daemon_id.clone(),
        reason,
    };
    // abs_diff can't overflow, whatever time a request claims.
    let skew_ms = now_ms.abs_diff(request.sent_unix_ms);
    if skew_ms > MAX_CLOCK_SKEW_MS {
        return Err(stale(format!(
            "its clock is {} seconds off this node's; at most {} are allowed",
            skew_ms / 1000,
            MAX_CLOCK_SKEW_MS / 1000
        )));
    }
    if request.daemon_nonce.len() < MIN_NONCE_BYTES {
        return Err(stale(format!(
            "its nonce has {} bytes; at least {MIN_NONCE_BYTES} are needed",
            request.daemon_nonce.len()
        )));
    }
    let mut table = lock_handshakes(state);
    table
        .recent_nonces
        .retain(|(_, seen_ms)| now_ms.abs_diff(*seen_ms) <= NONCE_MEMORY_MS);
    if table
        .recent_nonces
        .iter()
        .any(|(nonce, _)| nonce == &request.daemon_nonce)
    {
        return Err(stale("its nonce was used before".to_string()));
    }
    if table.recent_nonces.len() >= NONCE_MEMORY_LIMIT {
        table.recent_nonces.remove(0);
    }
    table.recent_nonces.push((request.daemon_nonce.clone(), now_ms));
    Ok(())
}

/// The format of paired_daemons.json; the reason when it doesn't match.
fn parse_paired_daemons(text: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("it isn't JSON: {error}"))?;
    let format_version = value.get(FORMAT_VERSION_KEY).and_then(Value::as_u64);
    if format_version != Some(PAIRED_DAEMONS_FORMAT_VERSION) {
        return Err(format!(
            "its {FORMAT_VERSION_KEY} isn't {PAIRED_DAEMONS_FORMAT_VERSION}"
        ));
    }
    let Some(entries) = value.get(DAEMON_IDS_KEY).and_then(Value::as_array) else {
        return Err(format!("it has no {DAEMON_IDS_KEY} list"));
    };
    let mut daemon_ids = Vec::new();
    for entry in entries {
        match entry.as_str() {
            Some(daemon_id) => daemon_ids.push(daemon_id.to_string()),
            None => return Err(format!("{DAEMON_IDS_KEY} holds {entry}, which isn't a text")),
        }
    }
    Ok(daemon_ids)
}

fn write_paired_daemons(path: &Path, daemon_ids: &[String]) -> Result<(), NodeError> {
    let document = json!({ FORMAT_VERSION_KEY: PAIRED_DAEMONS_FORMAT_VERSION, DAEMON_IDS_KEY: daemon_ids });
    let mut text = serde_json::to_string_pretty(&document).map_err(|error| unusable(path, error))?;
    text.push('\n');
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| unusable(path, format!("its folder can't be created: {error}")))?;
    }
    let temporary_path = temporary_path_beside(path)?;
    write_private_file(&temporary_path, text.as_bytes())?;
    let renamed = fs::rename(&temporary_path, path);
    if let Err(error) = renamed {
        let _ = fs::remove_file(&temporary_path);
        return Err(unusable(path, format!("can't be replaced: {error}")));
    }
    Ok(())
}

#[cfg(unix)]
fn check_owner_only_writes(path: &Path) -> Result<(), NodeError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).map_err(|error| unusable(path, format!("can't be read: {error}")))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & GROUP_AND_OTHER_WRITE_BITS != 0 {
        return Err(unusable(
            path,
            format!("its mode is {mode:04o}; only its owner may change it"),
        ));
    }
    Ok(())
}

/// Windows file access is set by ACLs, which the Windows build checks when it lands (tier 2).
#[cfg(not(unix))]
fn check_owner_only_writes(_path: &Path) -> Result<(), NodeError> {
    Ok(())
}

/// A text or byte field with its length first, so `("ab", "c")` and `("a", "bc")` sign different bytes.
fn push_field(bytes: &mut Vec<u8>, field: &[u8]) {
    let length = u32::try_from(field.len()).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(field);
}

fn lock_handshakes(state: &NodeState) -> std::sync::MutexGuard<'_, HandshakeTable> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state
        .handshakes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unusable(path: &Path, reason: impl std::fmt::Display) -> NodeError {
    NodeError::IdentityFileUnusable {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Barrier};

    use rastai_core::paths::DataPaths;
    use rastai_core::settings::{default_settings, node_settings, resolve_settings};
    use rastai_proto::common::v1::HardwareProfile;

    use crate::hardware::system_hardware_sources;
    use crate::identity::{NodeIdentity, daemon_id_from_public_key, load_or_create_identity, public_key_bytes};
    use crate::node::{NodeMode, NodeOptions, build_node_state};

    const HUB_SOCKET: &str = "/run/user/1000/rastai/node-hub.sock";
    const NOW_MS: i64 = 1_800_000_000_000;
    /// Writers racing to pin a daemon in one paired_daemons.json, and how many times they race.
    const RACING_WRITERS: usize = 8;
    const RACE_ROUNDS: usize = 50;
    static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

    /// A node in a fresh folder, and a daemon identity in the same folder. The folder is removed when the test ends,
    /// also when an assertion fails.
    struct Setup {
        node: NodeState,
        daemon: NodeIdentity,
        daemon_id: String,
        folder: PathBuf,
    }

    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.folder);
        }
    }

    fn setup() -> Setup {
        let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
        let folder = std::env::temp_dir().join(format!("rnhs-{}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let paths = DataPaths {
            config: folder.join("config"),
            data: folder.join("data"),
            cache: folder.join("cache"),
            state: folder.join("state"),
            runtime: folder.join("run"),
        };
        let options = NodeOptions {
            mode: NodeMode::Embedded,
            paths,
            settings: node_settings(&resolve_settings(&[default_settings()])),
            hardware_sources: system_hardware_sources(),
            jobs: crate::jobs::supervisor::default_job_options(Vec::new()),
        };
        let identity = load_or_create_identity(&folder.join("node.key")).unwrap();
        let node = build_node_state(options, identity, HardwareProfile::default());
        let daemon = load_or_create_identity(&folder.join("daemon.key")).unwrap();
        let daemon_id = daemon_id_from_public_key(&public_key_bytes(&daemon));
        Setup {
            node,
            daemon,
            daemon_id,
            folder,
        }
    }

    fn pin(setup: &Setup) {
        pin_daemon_key(&paired_daemons_path(&setup.node.paths), &setup.daemon_id).unwrap();
    }

    /// A request signed by `signer` for this node, sent at `sent_unix_ms` with `nonce`.
    fn signed_request(setup: &Setup, signer: &NodeIdentity, nonce: &[u8], sent_unix_ms: i64) -> HandshakeRequest {
        let signed = daemon_challenge_bytes(&setup.node.node_id, nonce, sent_unix_ms, HUB_SOCKET);
        HandshakeRequest {
            protocol_version: NODE_PROTOCOL_VERSION,
            app_version: APP_VERSION.to_string(),
            daemon_id: setup.daemon_id.clone(),
            daemon_nonce: nonce.to_vec(),
            sent_unix_ms,
            daemon_signature: sign_challenge(signer, &signed),
            hub_socket_path: HUB_SOCKET.to_string(),
            ..HandshakeRequest::default()
        }
    }

    fn refusal_code(setup: &Setup, request: &HandshakeRequest) -> &'static str {
        accept_handshake(&setup.node, request, NOW_MS).unwrap_err().code()
    }

    #[test]
    fn a_pinned_daemon_gets_a_session_the_daemon_can_check() {
        let setup = setup();
        pin(&setup);
        let request = signed_request(&setup, &setup.daemon, &[7; 32], now_unix_ms());
        let response = answer_handshake(&setup.node, &request);
        assert_eq!(response.diagnostic, None);
        assert_eq!(response.node_id, setup.node.node_id);
        assert_eq!(response.protocol_version, NODE_PROTOCOL_VERSION);
        assert!(!response.session_token.is_empty());
        // The daemon's side: the node's signature verifies with the node's key, over the token it was given.
        let node_key = public_key_bytes(&setup.node.identity);
        let signed = node_challenge_bytes(&setup.daemon_id, &[7; 32], &response.session_token);
        assert!(verify_challenge(&node_key, &signed, &response.node_signature).is_ok());
        let session = lock_handshakes(&setup.node).session.clone().unwrap();
        assert_eq!(session.session_token, response.session_token);
        assert_eq!(session.hub_socket_path, HUB_SOCKET);
    }

    // The version check comes first, so even an unpaired daemon learns the node's version and can offer an update.
    #[test]
    fn another_protocol_version_is_refused_with_both_versions() {
        let setup = setup();
        let mut request = signed_request(&setup, &setup.daemon, &[1; 32], NOW_MS);
        request.protocol_version = NODE_PROTOCOL_VERSION + 1;
        let response = answer_handshake(&setup.node, &request);
        let refusal = response.diagnostic.unwrap();
        assert_eq!(refusal.code, "RRS-N0001");
        assert_eq!(response.protocol_version, NODE_PROTOCOL_VERSION);
        assert!(response.session_token.is_empty());
        assert!(response.hardware.is_none());
    }

    #[test]
    fn an_unpinned_or_forged_daemon_is_refused() {
        let setup = setup();
        let request = signed_request(&setup, &setup.daemon, &[2; 32], NOW_MS);
        assert_eq!(refusal_code(&setup, &request), "RRS-N0002");
        pin(&setup);
        // Pinned id, but signed by another key: the caller doesn't hold the pinned key.
        let forger = load_or_create_identity(&setup.folder.join("forger.key")).unwrap();
        let forged = signed_request(&setup, &forger, &[3; 32], NOW_MS);
        assert_eq!(refusal_code(&setup, &forged), "RRS-N0002");
        // Signed by the pinned key, but changed afterwards: the hub socket isn't the one it signed.
        let mut changed = signed_request(&setup, &setup.daemon, &[4; 32], NOW_MS);
        changed.hub_socket_path = "/tmp/elsewhere.sock".to_string();
        assert_eq!(refusal_code(&setup, &changed), "RRS-N0002");
    }

    #[test]
    fn a_replayed_or_late_handshake_is_refused() {
        let setup = setup();
        pin(&setup);
        let request = signed_request(&setup, &setup.daemon, &[5; 32], NOW_MS);
        assert!(accept_handshake(&setup.node, &request, NOW_MS).is_ok());
        assert_eq!(refusal_code(&setup, &request), "RRS-N0007");
        let max_skew_ms = i64::try_from(MAX_CLOCK_SKEW_MS).unwrap();
        let late = signed_request(&setup, &setup.daemon, &[6; 32], NOW_MS - max_skew_ms - 1);
        assert_eq!(refusal_code(&setup, &late), "RRS-N0007");
        let early = signed_request(&setup, &setup.daemon, &[8; 32], NOW_MS + max_skew_ms + 1);
        assert_eq!(refusal_code(&setup, &early), "RRS-N0007");
        let short_nonce = signed_request(&setup, &setup.daemon, &[9; 4], NOW_MS);
        assert_eq!(refusal_code(&setup, &short_nonce), "RRS-N0007");
    }

    // A paired daemon could sign any time at all; the farthest ones are stale too, never an overflow.
    #[test]
    fn a_time_at_either_end_of_the_range_is_stale() {
        let setup = setup();
        pin(&setup);
        let far_past = signed_request(&setup, &setup.daemon, &[11; 32], i64::MIN);
        assert_eq!(refusal_code(&setup, &far_past), "RRS-N0007");
        let far_future = signed_request(&setup, &setup.daemon, &[12; 32], i64::MAX);
        assert_eq!(refusal_code(&setup, &far_future), "RRS-N0007");
    }

    // Writers at the same time each use a temporary file of their own, so none fails and none leaves one behind.
    #[test]
    fn writers_racing_to_pin_all_succeed() {
        let setup = setup();
        let start = Arc::new(Barrier::new(RACING_WRITERS));
        let mut writers = Vec::new();
        for _ in 0..RACING_WRITERS {
            let start = Arc::clone(&start);
            let root = setup.folder.join("pins");
            let daemon_id = setup.daemon_id.clone();
            writers.push(std::thread::spawn(move || {
                let mut results = Vec::new();
                for round in 0..RACE_ROUNDS {
                    let path = root.join(round.to_string()).join("paired_daemons.json");
                    start.wait();
                    results.push(pin_daemon_key(&path, &daemon_id));
                }
                results
            }));
        }
        for writer in writers {
            for result in writer.join().unwrap() {
                assert_eq!(result, Ok(()));
            }
        }
        for round in 0..RACE_ROUNDS {
            let folder = setup.folder.join("pins").join(round.to_string());
            let names: Vec<String> = fs::read_dir(&folder)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(names, ["paired_daemons.json"], "round {round}");
            let pinned = read_paired_daemons(&folder.join("paired_daemons.json")).unwrap();
            assert_eq!(pinned, std::slice::from_ref(&setup.daemon_id));
        }
    }

    // Length prefixes keep two different field lists from signing the same bytes.
    #[test]
    fn field_boundaries_change_the_signed_bytes() {
        assert_ne!(
            daemon_challenge_bytes("node_ab", b"c", 1, "d"),
            daemon_challenge_bytes("node_a", b"bc", 1, "d")
        );
        assert_ne!(
            node_challenge_bytes("daemon_a", b"n", "token-1"),
            node_challenge_bytes("daemon_a", b"n", "token-2")
        );
    }

    #[test]
    fn pinning_twice_keeps_one_entry_and_a_damaged_file_is_refused() {
        let setup = setup();
        let path = paired_daemons_path(&setup.node.paths);
        assert_eq!(read_paired_daemons(&path).unwrap(), Vec::<String>::new());
        pin(&setup);
        pin(&setup);
        assert_eq!(
            read_paired_daemons(&path).unwrap(),
            std::slice::from_ref(&setup.daemon_id)
        );
        assert_eq!(pin_daemon_key(&path, "daemon_short").unwrap_err().code(), "RRS-N0006");
        fs::write(&path, "{\"format_version\": 1}").unwrap();
        assert_eq!(read_paired_daemons(&path).unwrap_err().code(), "RRS-N0006");
        let request = signed_request(&setup, &setup.daemon, &[10; 32], NOW_MS);
        assert_eq!(refusal_code(&setup, &request), "RRS-N0006");
    }

    #[cfg(unix)]
    #[test]
    fn a_pin_file_others_can_change_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let setup = setup();
        pin(&setup);
        let path = paired_daemons_path(&setup.node.paths);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(read_paired_daemons(&path).unwrap_err().code(), "RRS-N0006");
    }
}
