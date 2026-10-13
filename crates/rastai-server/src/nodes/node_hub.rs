//! `NodeHubService`, the daemon's side of the calls nodes make (contracts 3.2 and 4.4; layout:
//! rust-control-plane.md 8.1), served on `node-hub.sock`. The embedded node dials it once the daemon has handshaken;
//! a remote node reaches it through its SSH session (M9).
//!
//! - **Every call carries the session token** in `rastai-session-token`. A call without the current session's token
//!   is refused with `RRS-N0009` as gRPC `UNAUTHENTICATED`: the call never ran, so there is no answer to put a
//!   diagnostic in (contracts 4). The node beats at once when a daemon handshakes, so that beat can arrive while the
//!   handshake's answer is still on its way back; a call for a node that is handshaking waits up to
//!   `HANDSHAKE_SETTLE_MS` for the session before it is judged.
//! - **`Heartbeat`** notes the beat (the link's watch counts missed ones, node_links.rs), publishes the telemetry as
//!   the ephemeral `node.heartbeat`, and turns the node's job list into `job.state_changed` records (reconcile.rs):
//!   the node's job states are in no log, so heartbeats are how the daemon learns them (contracts 7.3). The effects
//!   run on a task of their own: the node's hub client gives up on a call after a few seconds, and the server then
//!   drops the call's handler, which must never stop a beat halfway (a job's end recorded but its run never
//!   settled). The answer still waits for them, so a beat that was answered has been applied.
//! - **`ExecuteCommand`** answers `RRS-P1004` in M1: a call names its plugin only once plugin modules exist (M4), and
//!   the daemon runs no command without a principal.
//!
//! The handler holds the server weakly, so a connection the node keeps open never keeps a stopping daemon's store
//! from closing (shutdown.rs).

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use rastai_core::paths::node_hub_socket_path;
use rastai_core::time::now_unix_ms;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::diagnostic;
use rastai_diag::logger::log_warn;
use rastai_node::handshake::session_token_from_metadata;
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use rastai_proto::common::v1::{EventClass, NodeState, NodeTelemetry};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, EventPayload};
use rastai_proto::node::v1::node_hub_service_server::{NodeHubService, NodeHubServiceServer};
use rastai_proto::node::v1::{HeartbeatRequest, HeartbeatResponse};
use rastai_transport::grpc_channel::serve_grpc_on_socket;
use rastai_transport::local_socket::bind_local_socket;
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status};

use crate::error::ServerError;
use crate::events::bus::publish;
use crate::nodes::node_links::{link_state, node_of_session_token, note_heartbeat, session_token_is_current};
use crate::nodes::reconcile::{SnapshotScope, apply_job_snapshot};
use crate::server_state::Server;
use crate::shutdown::wait_until_stopping;

const CODE_SESSION_TOKEN_REFUSED: &str = "RRS-N0009";
/// How long a call for a handshaking node waits for the handshake to finish, and how often it looks.
const HANDSHAKE_SETTLE_MS: u64 = 2000;
const HANDSHAKE_SETTLE_POLL_MS: u64 = 10;
const CODE_PLUGIN_COMMAND_NOT_AVAILABLE: &str = "RRS-P1004";
const HEARTBEAT_CALL: &str = "NodeHubService.Heartbeat";
const EXECUTE_COMMAND_CALL: &str = "NodeHubService.ExecuteCommand";
const NODE_HEARTBEAT_EVENT: &str = "node.heartbeat";
/// The bus source of what a node reports (contracts 7.2).
const NODE_SOURCE_PREFIX: &str = "node:";

/// The `NodeHubService` handler: the server, held weakly (see the top).
struct NodeHubHandler {
    server: Weak<Server>,
}

/// Serves `NodeHubService` on `node-hub.sock` until the daemon stops. A stale socket from a crashed daemon is
/// replaced; the daemon lock guarantees it is no live daemon's.
pub fn serve_node_hub(server: &Arc<Server>) -> Result<JoinHandle<()>, ServerError> {
    let listener = bind_local_socket(&node_hub_socket_path(&server.paths))?;
    let handler = NodeHubHandler {
        server: Arc::downgrade(server),
    };
    let router = tonic::transport::Server::builder().add_service(NodeHubServiceServer::new(handler));
    let mut shutdown = server.shutdown.subscribe();
    let task = tokio::spawn(async move {
        tokio::select! {
            served = serve_grpc_on_socket(listener, router) => {
                if let Err(error) = served {
                    log_warn("node-hub.sock stopped serving", &[("code", error.code())]);
                }
            }
            () = wait_until_stopping(&mut shutdown) => {}
        }
    });
    Ok(task)
}

#[tonic::async_trait]
impl NodeHubService for NodeHubHandler {
    async fn heartbeat(&self, request: Request<HeartbeatRequest>) -> Result<Response<HeartbeatResponse>, Status> {
        let server = self.live_server()?;
        let session_token = session_token_from_metadata(request.metadata());
        let beat = request.into_inner();
        if !session_is_current(&server, &beat.node_id, session_token.as_deref()).await {
            return Err(token_refused(HEARTBEAT_CALL));
        }
        let node_id = beat.node_id.clone();
        let effects = tokio::spawn(async move { apply_heartbeat(&server, beat).await });
        if effects.await.is_err() {
            log_warn("a heartbeat's effects stopped halfway", &[("node_id", &node_id)]);
        }
        Ok(Response::new(HeartbeatResponse {}))
    }

    async fn execute_command(&self, request: Request<ExecuteRequest>) -> Result<Response<ExecuteResponse>, Status> {
        let server = self.live_server()?;
        let session_token = session_token_from_metadata(request.metadata());
        if node_of_session_token(&server, session_token.as_deref()).is_none() {
            return Err(token_refused(EXECUTE_COMMAND_CALL));
        }
        let command_id = request.into_inner().command_id;
        let refusal = diagnostic(CODE_PLUGIN_COMMAND_NOT_AVAILABLE, &[("command_id", &command_id)]);
        let response = ExecuteResponse {
            ok: false,
            diagnostics: vec![diagnostic_to_proto(&refusal)],
            ..ExecuteResponse::default()
        };
        Ok(Response::new(response))
    }
}

impl NodeHubHandler {
    /// The server while the daemon runs; a call that arrives while it stops is turned away.
    fn live_server(&self) -> Result<Arc<Server>, Status> {
        self.server
            .upgrade()
            .ok_or_else(|| Status::unavailable("the daemon is stopping"))
    }
}

/// True when the token is the node's current session's; while the node is handshaking, waits for it first (see
/// the top).
async fn session_is_current(server: &Server, node_id: &str, session_token: Option<&str>) -> bool {
    let deadline = Instant::now() + Duration::from_millis(HANDSHAKE_SETTLE_MS);
    loop {
        if session_token_is_current(server, node_id, session_token) {
            return true;
        }
        let handshaking = link_state(server, node_id) == Some(NodeState::Handshaking);
        if !handshaking || Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(HANDSHAKE_SETTLE_POLL_MS)).await;
    }
}

/// One heartbeat's effects, in this order: the telemetry on the bus, the link's clock and telemetry (what
/// `node.status` reads, so a reader that sees the beat there knows its bus event is out), then the job states.
pub async fn apply_heartbeat(server: &Arc<Server>, beat: HeartbeatRequest) {
    if let Some(telemetry) = beat.telemetry.clone() {
        publish_telemetry(server, &beat.node_id, telemetry);
    }
    note_heartbeat(server, &beat.node_id, beat.telemetry).await;
    apply_job_snapshot(server, &beat.node_id, &beat.jobs, SnapshotScope::TheseJobs).await;
}

/// `node.heartbeat`, ephemeral: the dashboard's telemetry (contracts 7.3).
fn publish_telemetry(server: &Server, node_id: &str, telemetry: NodeTelemetry) {
    let event = BusEvent {
        bus_sequence: 0,
        event_type: NODE_HEARTBEAT_EVENT.to_string(),
        event_class: EventClass::Ephemeral as i32,
        subject_id: node_id.to_string(),
        source: format!("{NODE_SOURCE_PREFIX}{node_id}"),
        emitted_unix_ms: now_unix_ms(),
        payload: Some(EventPayload {
            payload: Some(Payload::NodeHeartbeat(telemetry)),
        }),
    };
    publish(&server.bus, event);
}

/// `RRS-N0009` as the call's gRPC status, its code first so the node's log names it.
fn token_refused(call: &str) -> Status {
    let refusal = diagnostic(CODE_SESSION_TOKEN_REFUSED, &[("service", call)]);
    Status::unauthenticated(format!("{CODE_SESSION_TOKEN_REFUSED}: {}", refusal.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_call_names_its_code_and_the_call() {
        let status = token_refused(HEARTBEAT_CALL);
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
        assert!(status.message().starts_with("RRS-N0009: "), "{}", status.message());
        assert!(status.message().contains(HEARTBEAT_CALL), "{}", status.message());
    }
}
