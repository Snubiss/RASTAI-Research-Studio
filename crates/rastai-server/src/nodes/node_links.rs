//! The daemon's links to its nodes (layout: rust-control-plane.md 8.1 and 8.3; flows/platform.md 2.1 and 2.2;
//! contracts 3.2, 4.4 and 9.2): one `NodeLink` per node, its connection state, the handshake, and every `NodeService`
//! call the daemon makes. M1 has one node, the embedded one, reached over node.sock exactly as a remote one would be.
//!
//! - **The handshake** signs a fresh challenge with daemon.key (the node checks it against its pinned keys), then
//!   checks the node's answer: the node id it expects, the protocol version, and the node's signature over the
//!   session token with the key the node id names (`RRS-N0002` otherwise). The token goes on every later call in the
//!   `rastai-session-token` metadata key, and the hub (node_hub.rs) takes only calls that carry it.
//! - **States** (contracts 9.2): connecting → handshaking → connected. A refused protocol version is `incompatible`
//!   (`RRS-N0001`); any other failure goes back to connecting. Each change is written to the node's `nodes` row and
//!   recorded as `node.state_changed` in the daemon log.
//! - **Heartbeats keep a link alive.** `watch_node_links` marks a node unreachable after
//!   `nodes.missed_heartbeats_unreachable` missed beats (`RRS-N2001`) and its running jobs lost, and connects again,
//!   1 s doubling to 30 s; each connect reconciles (reconcile.rs).
//! - **`NodeConnection`** holds the link's current channel and token. It is the node's `RunEventSource`: one object
//!   for the daemon's life that a reconnect gives a new channel, so a run tail re-watches through it (run_tail.rs).
//!
//! Only this file and node_hub.rs use tonic (contracts 2.5); everything else calls the functions here.

// Every function here answers a catalog `Diagnostic` on failure, which is larger than clippy's limit for an error
// type; boxing it would only add indirection to code the owner reads.
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rastai_core::paths::node_hub_socket_path;
use rastai_core::random::random_bytes;
use rastai_core::settings::setting_u64;
use rastai_core::states::node_state_name;
use rastai_core::time::now_unix_ms;
use rastai_core::versions::{APP_VERSION, NODE_PROTOCOL_VERSION};
use rastai_core::wire::diagnostic_from_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::{log_info, log_warn};
use rastai_node::handshake::{daemon_challenge_bytes, node_challenge_bytes, request_with_session_token};
use rastai_node::identity::{
    daemon_id_from_public_key, public_key_bytes, public_key_from_id, sign_challenge, verify_challenge,
};
use rastai_proto::common::v1::{NodeState, NodeTelemetry};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{EventPayload, EventRecord, NodeStateChanged};
use rastai_proto::node::v1::node_service_client::NodeServiceClient;
use rastai_proto::node::v1::{
    HandshakeRequest, HandshakeResponse, JobInfo, JobSignal, JobSpec, ListJobsRequest, SignalJobRequest,
    WatchRunEventsRequest,
};
use rastai_store::index::kept_tables::{NodeRow, get_node};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use rastai_transport::error::TransportError;
use rastai_transport::grpc_channel::connect_grpc_channel;
use tokio::sync::mpsc;
use tonic::transport::Channel;

use crate::events::daemon_events::record_daemon_event;
use crate::indexing::run_tail::RunEventSource;
use crate::nodes::reconcile::{mark_running_jobs_lost, reconcile_node};
use crate::server_state::Server;
use crate::shutdown::wait_until_stopping;

/// The daemon's nonce: 32 random bytes (the node refuses fewer than 16).
const NONCE_BYTES: usize = 32;
/// How long one unary `NodeService` call or a dial may take before it counts as failed.
const NODE_CALL_TIMEOUT_SECONDS: u64 = 30;
/// Reconnect backoff (contracts 9.2, `NODE_RECONNECT_BACKOFF_SECONDS`): 1 s doubling to 30 s.
const RECONNECT_FIRST_BACKOFF_SECONDS: u64 = 1;
const RECONNECT_MAX_BACKOFF_SECONDS: u64 = 30;
/// How often the watch looks at heartbeats and due reconnects.
const WATCH_TICK_MS: u64 = 500;
/// How many records a run-event stream holds before the forwarding task waits for the tail to take them.
const RUN_EVENT_CHANNEL_CAPACITY: usize = 64;
const HEARTBEAT_SECONDS_SETTING: &str = "nodes.heartbeat_seconds";
const MISSED_HEARTBEATS_SETTING: &str = "nodes.missed_heartbeats_unreachable";
const NODE_ID_PREFIX: &str = "node_";
/// `nodes.transport` of the embedded node (contracts 8.2).
const LOCAL_TRANSPORT: &str = "local";
/// `nodes.name` of the embedded node: the machine the daemon runs on.
const EMBEDDED_NODE_NAME: &str = "this machine";
const NODE_STATE_CHANGED_EVENT: &str = "node.state_changed";
const CODE_PROTOCOL_VERSION: &str = "RRS-N0001";
const CODE_NOT_PAIRED: &str = "RRS-N0002";
const CODE_UNREACHABLE: &str = "RRS-N2001";
const CODE_CONNECTION_DROPPED: &str = "RRS-N2002";
const CODE_RECONNECTED: &str = "RRS-N2003";
const CODE_CONNECTION_FAILED: &str = "RRS-N2008";
const CODE_NO_SUCH_NODE: &str = "RRS-N0008";

/// How the daemon reaches a node. SSH joins with remote nodes (M9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTransport {
    Embedded { socket_path: PathBuf },
}

/// One node the daemon knows, and where its connection stands.
pub struct NodeLink {
    pub node_id: String,
    pub state: NodeState,
    pub transport: NodeTransport,
    /// The node's own protocol version and capabilities, from its last handshake.
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    /// The token of the current session; empty before the first handshake.
    pub session_token: String,
    /// When the last heartbeat arrived; the connect counts as one.
    pub last_heartbeat_unix_ms: i64,
    /// The last heartbeat's telemetry, for `node.status`.
    pub telemetry: Option<NodeTelemetry>,
    /// When the watch may try to connect again, and the pause after that.
    pub retry_at_unix_ms: i64,
    pub retry_backoff_seconds: u64,
    pub connection: Arc<NodeConnection>,
}

/// Every link, behind one lock that is never held across an `.await` (`Server.nodes`).
#[derive(Default)]
pub struct NodeLinks {
    pub links: Mutex<Vec<NodeLink>>,
}

/// The channel to one node and the session token its calls carry. It outlives reconnects (see the top).
pub struct NodeConnection {
    node_id: String,
    channel: Mutex<Option<NodeChannel>>,
}

#[derive(Clone)]
struct NodeChannel {
    client: NodeServiceClient<Channel>,
    session_token: String,
}

/// The links of a daemon whose embedded node has this id and listens on `socket_path`.
pub fn embedded_node_links(node_id: &str, socket_path: &Path) -> NodeLinks {
    let link = NodeLink {
        node_id: node_id.to_string(),
        state: NodeState::Connecting,
        transport: NodeTransport::Embedded {
            socket_path: socket_path.to_path_buf(),
        },
        protocol_version: 0,
        capabilities: Vec::new(),
        session_token: String::new(),
        last_heartbeat_unix_ms: 0,
        telemetry: None,
        retry_at_unix_ms: 0,
        retry_backoff_seconds: RECONNECT_FIRST_BACKOFF_SECONDS,
        connection: Arc::new(NodeConnection {
            node_id: node_id.to_string(),
            channel: Mutex::new(None),
        }),
    };
    NodeLinks {
        links: Mutex::new(vec![link]),
    }
}

/// The embedded node's id.
pub fn embedded_node_id(server: &Server) -> String {
    let links = lock_links(server);
    let embedded = links
        .iter()
        .find(|link| matches!(link.transport, NodeTransport::Embedded { .. }));
    embedded.map(|link| link.node_id.clone()).unwrap_or_default()
}

pub fn node_is_known(server: &Server, node_id: &str) -> bool {
    lock_links(server).iter().any(|link| link.node_id == node_id)
}

pub fn link_is_connected(server: &Server, node_id: &str) -> bool {
    link_state(server, node_id) == Some(NodeState::Connected)
}

pub fn link_state(server: &Server, node_id: &str) -> Option<NodeState> {
    with_link(server, node_id, |link| link.state)
}

/// The node's last heartbeat telemetry, if one arrived.
pub fn link_telemetry(server: &Server, node_id: &str) -> Option<NodeTelemetry> {
    with_link(server, node_id, |link| link.telemetry.clone()).flatten()
}

/// True when `session_token` is the current session's token of a connected node (contracts 4.4).
pub fn session_token_is_current(server: &Server, node_id: &str, session_token: Option<&str>) -> bool {
    let Some(session_token) = session_token else {
        return false;
    };
    let current = with_link(server, node_id, |link| {
        link.state == NodeState::Connected && link.session_token == session_token
    });
    current == Some(true)
}

/// The connected node whose current session `session_token` is; None for no token or a stale one.
pub fn node_of_session_token(server: &Server, session_token: Option<&str>) -> Option<String> {
    let session_token = session_token?;
    let links = lock_links(server);
    let current = links
        .iter()
        .find(|link| link.state == NodeState::Connected && link.session_token == session_token);
    current.map(|link| link.node_id.clone())
}

/// Notes a heartbeat that arrived now, and writes its time to the node's row.
pub async fn note_heartbeat(server: &Server, node_id: &str, telemetry: Option<NodeTelemetry>) {
    let now = now_unix_ms();
    with_link(server, node_id, |link| {
        link.last_heartbeat_unix_ms = now;
        link.telemetry = telemetry;
    });
    write_node_row(server, node_id).await;
}

/// The node's run events, through its connection (`RunEventSource`).
pub fn run_event_source(server: &Server, node_id: &str) -> Option<Arc<dyn RunEventSource>> {
    let connection = with_link(server, node_id, |link| link.connection.clone())?;
    Some(connection)
}

/// Writes every link's `nodes` row as it stands now: at startup, before any record names the node.
pub async fn write_node_rows(server: &Server) {
    let node_ids: Vec<String> = lock_links(server).iter().map(|link| link.node_id.clone()).collect();
    for node_id in node_ids {
        write_node_row(server, &node_id).await;
    }
}

/// Dials the node, handshakes, and reconciles (contracts 9.2: connecting → handshaking → connected). On failure the
/// link is back in connecting, or incompatible for a protocol version this daemon doesn't speak, and the diagnostic
/// says why; the watch retries.
pub async fn connect_node(server: &Arc<Server>, node_id: &str) -> Result<(), Diagnostic> {
    let Some(socket_path) = with_link(server, node_id, |link| link_socket_path(link)) else {
        return Err(diagnostic(CODE_NO_SUCH_NODE, &[("node_id", node_id)]));
    };
    let was_unreachable = link_state(server, node_id) == Some(NodeState::Unreachable);
    move_link_state(server, node_id, NodeState::Connecting).await;
    let channel = match dial(&socket_path).await {
        Ok(channel) => channel,
        Err(problem) => return Err(problem),
    };
    move_link_state(server, node_id, NodeState::Handshaking).await;
    let mut client = NodeServiceClient::new(channel);
    let answer = match handshake(server, node_id, &mut client, &socket_path).await {
        Ok(answer) => answer,
        Err(problem) => {
            let next = state_after_refusal(&problem);
            move_link_state(server, node_id, next).await;
            return Err(problem);
        }
    };
    keep_session(server, node_id, client, &answer);
    move_link_state(server, node_id, NodeState::Connected).await;
    log_connected(node_id, was_unreachable);
    reconcile_node(server, node_id).await;
    Ok(())
}

/// `NodeService.StartJob`: the node's answer is the job's own state, `failed` included (contracts 4).
pub async fn start_job_on_node(server: &Server, node_id: &str, spec: JobSpec) -> Result<JobInfo, Diagnostic> {
    let mut channel = current_channel(server, node_id)?;
    let request = session_request(spec, &channel, node_id)?;
    let answered = timed(channel.client.start_job(request)).await;
    let info = call_result(answered, node_id)?;
    Ok(info)
}

/// `NodeService.SignalJob`. A signal the node didn't deliver is its `RRS-J0004` diagnostic.
pub async fn signal_job_on_node(
    server: &Server,
    node_id: &str,
    job_id: &str,
    signal: JobSignal,
) -> Result<(), Diagnostic> {
    let mut channel = current_channel(server, node_id)?;
    let message = SignalJobRequest {
        job_id: job_id.to_string(),
        signal: signal as i32,
    };
    let request = session_request(message, &channel, node_id)?;
    let answered = timed(channel.client.signal_job(request)).await;
    let response = call_result(answered, node_id)?;
    match response.diagnostic {
        Some(refusal) => Err(diagnostic_from_proto(&refusal)),
        None => Ok(()),
    }
}

/// `NodeService.ListJobs`: every job the node knows, for reconciliation (contracts 4.4).
pub async fn list_jobs_on_node(server: &Server, node_id: &str) -> Result<Vec<JobInfo>, Diagnostic> {
    let mut channel = current_channel(server, node_id)?;
    let request = session_request(ListJobsRequest {}, &channel, node_id)?;
    let answered = timed(channel.client.list_jobs(request)).await;
    let response = call_result(answered, node_id)?;
    Ok(response.jobs)
}

/// Until the daemon stops: marks a node unreachable once its heartbeats stop, and connects again when a retry is
/// due (see the top).
pub async fn watch_node_links(server: Arc<Server>) {
    let mut shutdown = server.shutdown.subscribe();
    loop {
        check_heartbeats(&server).await;
        reconnect_due_links(&server).await;
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(WATCH_TICK_MS)) => {}
            () = wait_until_stopping(&mut shutdown) => return,
        }
    }
}

// ---- Connecting ----

async fn dial(socket_path: &Path) -> Result<Channel, Diagnostic> {
    match tokio::time::timeout(call_timeout(), connect_grpc_channel(socket_path)).await {
        Ok(Ok(channel)) => Ok(channel),
        Ok(Err(error)) => Err(transport_diagnostic(&error, socket_path)),
        Err(_) => Err(connection_failed(
            socket_path,
            "the node didn't answer the dial in time",
        )),
    }
}

/// Sends the signed handshake and checks the answer (see the top).
async fn handshake(
    server: &Server,
    node_id: &str,
    client: &mut NodeServiceClient<Channel>,
    socket_path: &Path,
) -> Result<HandshakeResponse, Diagnostic> {
    let nonce = random_bytes(NONCE_BYTES).map_err(|error| connection_failed(socket_path, &format!("{error:?}")))?;
    let request = handshake_request(server, node_id, nonce.clone());
    let answer = match timed(client.handshake(request)).await {
        Ok(Ok(response)) => response.into_inner(),
        Ok(Err(status)) => return Err(connection_failed(socket_path, status.message())),
        Err(_) => {
            return Err(connection_failed(
                socket_path,
                "the node didn't answer the handshake in time",
            ));
        }
    };
    check_handshake_answer(server, node_id, &nonce, &answer)?;
    Ok(answer)
}

fn handshake_request(server: &Server, node_id: &str, nonce: Vec<u8>) -> HandshakeRequest {
    let hub_socket_path = node_hub_socket_path(&server.paths).to_string_lossy().into_owned();
    let sent_unix_ms = now_unix_ms();
    let signed = daemon_challenge_bytes(node_id, &nonce, sent_unix_ms, &hub_socket_path);
    HandshakeRequest {
        protocol_version: NODE_PROTOCOL_VERSION,
        capabilities: Vec::new(),
        app_version: APP_VERSION.to_string(),
        daemon_id: daemon_id(server),
        daemon_nonce: nonce,
        sent_unix_ms,
        daemon_signature: sign_challenge(&server.identity, &signed),
        hub_socket_path,
    }
}

/// The node's refusal, a node that isn't the one expected, another protocol version, or a signature that isn't the
/// node's own: each refuses the session.
fn check_handshake_answer(
    server: &Server,
    node_id: &str,
    nonce: &[u8],
    answer: &HandshakeResponse,
) -> Result<(), Diagnostic> {
    if let Some(refusal) = &answer.diagnostic {
        return Err(diagnostic_from_proto(refusal));
    }
    if answer.protocol_version != NODE_PROTOCOL_VERSION {
        let version = answer.protocol_version.to_string();
        let supported = NODE_PROTOCOL_VERSION.to_string();
        let values = [
            ("node_id", node_id),
            ("version", version.as_str()),
            ("supported_versions", supported.as_str()),
        ];
        return Err(diagnostic(CODE_PROTOCOL_VERSION, &values));
    }
    let not_paired = || diagnostic(CODE_NOT_PAIRED, &[("node_id", node_id)]);
    if answer.node_id != node_id {
        return Err(not_paired());
    }
    let Some(public_key) = public_key_from_id(node_id, NODE_ID_PREFIX) else {
        return Err(not_paired());
    };
    let signed = node_challenge_bytes(&daemon_id(server), nonce, &answer.session_token);
    verify_challenge(&public_key, &signed, &answer.node_signature).map_err(|_| not_paired())
}

/// A protocol version the daemon doesn't speak waits for `node.update` (incompatible); anything else is retried.
fn state_after_refusal(problem: &Diagnostic) -> NodeState {
    if problem.code == CODE_PROTOCOL_VERSION {
        NodeState::Incompatible
    } else {
        NodeState::Connecting
    }
}

fn keep_session(server: &Server, node_id: &str, client: NodeServiceClient<Channel>, answer: &HandshakeResponse) {
    let now = now_unix_ms();
    with_link(server, node_id, |link| {
        link.protocol_version = answer.protocol_version;
        link.capabilities = answer.capabilities.clone();
        link.session_token = answer.session_token.clone();
        link.last_heartbeat_unix_ms = now;
        link.retry_backoff_seconds = RECONNECT_FIRST_BACKOFF_SECONDS;
        let channel = NodeChannel {
            client,
            session_token: answer.session_token.clone(),
        };
        *lock_channel(&link.connection) = Some(channel);
    });
}

fn log_connected(node_id: &str, was_unreachable: bool) {
    if was_unreachable {
        let reconnected = diagnostic(CODE_RECONNECTED, &[("node_id", node_id)]);
        log_info(&reconnected.message, &[("code", CODE_RECONNECTED)]);
    } else {
        log_info("node connected", &[("node_id", node_id)]);
    }
}

fn daemon_id(server: &Server) -> String {
    daemon_id_from_public_key(&public_key_bytes(&server.identity))
}

// ---- The watch ----

/// A connected node whose last heartbeat is older than the allowed misses is unreachable, and its running jobs lost.
async fn check_heartbeats(server: &Arc<Server>) {
    let (heartbeat_ms, missed) = heartbeat_limits(server);
    let now = now_unix_ms();
    let silent: Vec<String> = lock_links(server)
        .iter()
        .filter(|link| link.state == NodeState::Connected)
        .filter(|link| now - link.last_heartbeat_unix_ms > heartbeat_ms.saturating_mul(missed))
        .map(|link| link.node_id.clone())
        .collect();
    for node_id in silent {
        let missed_text = missed.to_string();
        let lost = diagnostic(
            CODE_UNREACHABLE,
            &[("node_id", &node_id), ("missed_heartbeats", &missed_text)],
        );
        log_warn(&lost.message, &[("code", CODE_UNREACHABLE)]);
        forget_channel(server, &node_id);
        move_link_state(server, &node_id, NodeState::Unreachable).await;
        mark_running_jobs_lost(server, &node_id).await;
    }
}

/// Connects every link that is due: not connected, not incompatible, and past its backoff.
async fn reconnect_due_links(server: &Arc<Server>) {
    let now = now_unix_ms();
    let due: Vec<String> = lock_links(server)
        .iter()
        .filter(|link| matches!(link.state, NodeState::Connecting | NodeState::Unreachable))
        .filter(|link| link.retry_at_unix_ms <= now)
        .map(|link| link.node_id.clone())
        .collect();
    for node_id in due {
        if let Err(problem) = connect_node(server, &node_id).await {
            log_warn(
                "a node couldn't be connected; it is tried again",
                &[
                    ("node_id", &node_id),
                    ("code", &problem.code),
                    ("message", &problem.message),
                ],
            );
            schedule_retry(server, &node_id);
        }
    }
}

fn schedule_retry(server: &Server, node_id: &str) {
    let now = now_unix_ms();
    with_link(server, node_id, |link| {
        let pause_ms = i64::try_from(link.retry_backoff_seconds * 1000).unwrap_or(i64::MAX);
        link.retry_at_unix_ms = now.saturating_add(pause_ms);
        link.retry_backoff_seconds = (link.retry_backoff_seconds * 2).min(RECONNECT_MAX_BACKOFF_SECONDS);
    });
}

/// `nodes.heartbeat_seconds` in milliseconds and `nodes.missed_heartbeats_unreachable`, read at each tick so a
/// settings change applies at once.
fn heartbeat_limits(server: &Server) -> (i64, i64) {
    let settings = match server.settings.read() {
        Ok(settings) => settings,
        Err(poisoned) => poisoned.into_inner(),
    };
    let heartbeat_ms = setting_u64(&settings, HEARTBEAT_SECONDS_SETTING)
        .saturating_mul(1000)
        .max(1000);
    let missed = setting_u64(&settings, MISSED_HEARTBEATS_SETTING).max(1);
    (
        i64::try_from(heartbeat_ms).unwrap_or(i64::MAX),
        i64::try_from(missed).unwrap_or(i64::MAX),
    )
}

// ---- State and the nodes row ----

/// Moves the link to `to`; a real change writes the node's row and records `node.state_changed`.
async fn move_link_state(server: &Server, node_id: &str, to: NodeState) {
    let from = with_link(server, node_id, |link| std::mem::replace(&mut link.state, to));
    let Some(from) = from else {
        return;
    };
    if from == to {
        return;
    }
    write_node_row(server, node_id).await;
    let change = NodeStateChanged {
        node_id: node_id.to_string(),
        from_state: from as i32,
        to_state: to as i32,
    };
    let record = EventRecord {
        event_type: NODE_STATE_CHANGED_EVENT.to_string(),
        payload: Some(EventPayload {
            payload: Some(Payload::NodeStateChanged(change)),
        }),
        ..EventRecord::default()
    };
    if let Err(error) = record_daemon_event(server, record).await {
        log_warn(
            "a node's state change couldn't be recorded",
            &[("node_id", node_id), ("code", error.code())],
        );
    }
}

/// The node's `nodes` row as the link stands; the name and address a row already has are kept.
async fn write_node_row(server: &Server, node_id: &str) {
    let Some(mut row) = with_link(server, node_id, node_row_of) else {
        return;
    };
    if let Some(stored) = stored_node_row(server, node_id) {
        row.name = stored.name;
        if row.last_heartbeat_unix_ms.is_none() {
            row.last_heartbeat_unix_ms = stored.last_heartbeat_unix_ms;
        }
    }
    if let Err(error) = write_index(&server.store.index, IndexWrite::UpsertNode(row)).await {
        log_warn(
            "a node's row couldn't be written",
            &[("node_id", node_id), ("code", error.code())],
        );
    }
}

fn node_row_of(link: &mut NodeLink) -> NodeRow {
    let last_heartbeat = (link.last_heartbeat_unix_ms > 0).then_some(link.last_heartbeat_unix_ms);
    NodeRow {
        node_id: link.node_id.clone(),
        name: EMBEDDED_NODE_NAME.to_string(),
        transport: LOCAL_TRANSPORT.to_string(),
        address: String::new(),
        state: node_state_name(link.state).to_string(),
        protocol_version: i64::from(link.protocol_version),
        capabilities: link.capabilities.clone(),
        last_heartbeat_unix_ms: last_heartbeat,
    }
}

fn stored_node_row(server: &Server, node_id: &str) -> Option<NodeRow> {
    let reader = open_index_reader(&server.store.index_readers).ok()?;
    get_node(&reader, node_id).ok().flatten()
}

// ---- Calls ----

/// The node's channel, or `RRS-N2002` when it isn't connected.
fn current_channel(server: &Server, node_id: &str) -> Result<NodeChannel, Diagnostic> {
    let channel = with_link(server, node_id, |link| lock_channel(&link.connection).clone()).flatten();
    channel.ok_or_else(|| connection_dropped(node_id))
}

fn forget_channel(server: &Server, node_id: &str) {
    with_link(server, node_id, |link| {
        *lock_channel(&link.connection) = None;
        link.session_token.clear();
    });
}

fn session_request<T>(message: T, channel: &NodeChannel, node_id: &str) -> Result<tonic::Request<T>, Diagnostic> {
    request_with_session_token(message, &channel.session_token).map_err(|reason| {
        log_warn("a node call couldn't carry its session token", &[("reason", &reason)]);
        connection_dropped(node_id)
    })
}

/// A call's answer, or `RRS-N2002` when the connection failed or timed out under it.
fn call_result<T>(
    answered: Result<Result<tonic::Response<T>, tonic::Status>, tokio::time::error::Elapsed>,
    node_id: &str,
) -> Result<T, Diagnostic> {
    match answered {
        Ok(Ok(response)) => Ok(response.into_inner()),
        Ok(Err(status)) => {
            log_warn(
                "a node call failed",
                &[("node_id", node_id), ("status", status.message())],
            );
            Err(connection_dropped(node_id))
        }
        Err(_) => {
            log_warn("a node call timed out", &[("node_id", node_id)]);
            Err(connection_dropped(node_id))
        }
    }
}

async fn timed<F: std::future::Future>(call: F) -> Result<F::Output, tokio::time::error::Elapsed> {
    tokio::time::timeout(call_timeout(), call).await
}

fn call_timeout() -> Duration {
    Duration::from_secs(NODE_CALL_TIMEOUT_SECONDS)
}

impl RunEventSource for NodeConnection {
    fn node_id(&self) -> &str {
        &self.node_id
    }

    /// M1's one node is the embedded one, which shares the run folders; SSH nodes (M9) are remote.
    fn is_remote(&self) -> bool {
        false
    }

    fn watch_run_events(&self, run_id: &str, after_sequence: u64) -> mpsc::Receiver<Result<EventRecord, Diagnostic>> {
        let (sender, receiver) = mpsc::channel(RUN_EVENT_CHANNEL_CAPACITY);
        let channel = lock_channel(self).clone();
        let request = WatchRunEventsRequest {
            run_id: run_id.to_string(),
            after_sequence,
        };
        tokio::spawn(forward_run_events(self.node_id.clone(), channel, request, sender));
        receiver
    }
}

/// Hands the stream's records to the tail. The channel closes when the node ends the stream; an `Err` says it
/// broke, and the tail watches again later (run_tail.rs).
async fn forward_run_events(
    node_id: String,
    channel: Option<NodeChannel>,
    request: WatchRunEventsRequest,
    sender: mpsc::Sender<Result<EventRecord, Diagnostic>>,
) {
    let Some(mut channel) = channel else {
        let _ = sender.send(Err(connection_dropped(&node_id))).await;
        return;
    };
    let request = match session_request(request, &channel, &node_id) {
        Ok(request) => request,
        Err(problem) => {
            let _ = sender.send(Err(problem)).await;
            return;
        }
    };
    let mut stream = match channel.client.watch_run_events(request).await {
        Ok(response) => response.into_inner(),
        Err(_) => {
            let _ = sender.send(Err(connection_dropped(&node_id))).await;
            return;
        }
    };
    loop {
        let item = match stream.message().await {
            Ok(Some(record)) => Ok(record),
            Ok(None) => return,
            Err(_) => Err(connection_dropped(&node_id)),
        };
        let broke = item.is_err();
        if sender.send(item).await.is_err() || broke {
            return;
        }
    }
}

// ---- Small helpers ----

fn link_socket_path(link: &NodeLink) -> PathBuf {
    match &link.transport {
        NodeTransport::Embedded { socket_path } => socket_path.clone(),
    }
}

fn connection_dropped(node_id: &str) -> Diagnostic {
    diagnostic(CODE_CONNECTION_DROPPED, &[("node_id", node_id)])
}

fn connection_failed(socket_path: &Path, reason: &str) -> Diagnostic {
    let path = socket_path.display().to_string();
    diagnostic(CODE_CONNECTION_FAILED, &[("path", &path), ("reason", reason)])
}

/// A dial's failure with its own code (`RRS-N2004` nothing listening, `RRS-N2005` an unsafe folder, ...).
fn transport_diagnostic(error: &TransportError, socket_path: &Path) -> Diagnostic {
    let path = socket_path.display().to_string();
    let reason = format!("{error:?}");
    diagnostic(
        error.code(),
        &[("path", &path), ("reason", &reason), ("platform", std::env::consts::OS)],
    )
}

fn with_link<T>(server: &Server, node_id: &str, change: impl FnOnce(&mut NodeLink) -> T) -> Option<T> {
    let mut links = lock_links(server);
    links.iter_mut().find(|link| link.node_id == node_id).map(change)
}

fn lock_links(server: &Server) -> MutexGuard<'_, Vec<NodeLink>> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    server.nodes.links.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock_channel(connection: &NodeConnection) -> MutexGuard<'_, Option<NodeChannel>> {
    connection.channel.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_protocol_version_refusal_waits_for_an_update() {
        let refused = diagnostic(
            CODE_PROTOCOL_VERSION,
            &[("node_id", "node_a"), ("version", "0"), ("supported_versions", "1")],
        );
        assert_eq!(state_after_refusal(&refused), NodeState::Incompatible);
        let not_paired = diagnostic(CODE_NOT_PAIRED, &[("node_id", "node_a")]);
        assert_eq!(state_after_refusal(&not_paired), NodeState::Connecting);
    }

    #[test]
    fn a_dial_failure_keeps_the_transport_code_and_its_path() {
        let error = TransportError::SocketNotReachable {
            socket_path: PathBuf::from("/run/rastai/node.sock"),
            reason: "no such file".to_string(),
        };
        let problem = transport_diagnostic(&error, Path::new("/run/rastai/node.sock"));
        assert_eq!(problem.code, "RRS-N2004");
        assert!(problem.message.contains("/run/rastai/node.sock"), "{}", problem.message);
    }
}
