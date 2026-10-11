//! `run_node`, the node's thin entry, and `NodeState`, every table the node keeps (layout:
//! rust-node-gateway-proto.md sections 7.2 and 7.3). The embedded node (inside the daemon, on its own tokio runtime)
//! and `rastai node serve` run the same function; `NodeMode` is the only difference.
//!
//! The start-up order of layout 7.3 is spelled out in `run_node`. The steps later tasks build are named where they
//! go: serving WorkerHostService (M1-43), reattaching jobs and heartbeats (M1-45), llama-server (M3) and sandboxes
//! (M5).

use std::sync::{Arc, Mutex};

use rastai_core::paths::{DataPaths, node_key_path, node_socket_path};
use rastai_core::settings::NodeSettings;
use rastai_diag::logger::log_diagnostic;
use rastai_proto::common::v1::{HardwareProfile, NodeTelemetry};
use rastai_proto::node::v1::node_service_server::NodeServiceServer;
use rastai_transport::grpc_channel::serve_grpc_on_socket;
use rastai_transport::local_socket::bind_local_socket;
use tonic::transport::Server;

use crate::error::NodeError;
use crate::handshake::HandshakeTable;
use crate::hardware::{DetectedHardware, HardwareSources, detect_hardware};
use crate::identity::{NodeIdentity, load_or_create_identity, public_key_bytes};
use crate::node_service::NodeServiceHandler;
use crate::telemetry::start_telemetry;

/// What starts a node.
pub struct NodeOptions {
    pub mode: NodeMode,
    pub paths: DataPaths,
    pub settings: NodeSettings,
    /// Where hardware detection and telemetry read: `system_hardware_sources()` on a real machine.
    pub hardware_sources: HardwareSources,
}

/// Where the node runs. It decides where logs go and whether UpdateNode is allowed (standalone only, M9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeMode {
    Embedded,
    Standalone,
}

/// The node's state. Each table sits behind its own lock, which is never held across an `.await`.
pub struct NodeState {
    pub node_id: String,
    pub identity: NodeIdentity,
    pub mode: NodeMode,
    pub paths: DataPaths,
    pub settings: Mutex<NodeSettings>,
    pub hardware: Mutex<HardwareProfile>,
    pub telemetry: Mutex<NodeTelemetry>,
    pub handshakes: Mutex<HandshakeTable>,
}

/// Starts the node and serves NodeService on node.sock until the server stops.
pub async fn run_node(options: NodeOptions) -> Result<(), NodeError> {
    refuse_second_node(&options).await?;
    let identity = load_or_create_identity(&node_key_path(&options.paths))?;
    let detected = detect_hardware(&options.hardware_sources).await;
    log_hardware_notes(&detected);
    let hardware_sources = options.hardware_sources.clone();
    let nvidia_gpus = detected.nvidia_gpus.clone();
    let amd_gpus = detected.amd_gpus.clone();
    let state = Arc::new(build_node_state(options, identity, detected.profile));
    // serve_worker_host (M1-43) goes here, first, so live jobs can re-Register; then reattach_jobs (M1-45),
    // reattach_llama_servers (M3) and adopt_and_sweep_sandboxes (M5).
    // The loops stop when `_telemetry` is dropped: when serving ends or fails, or this future is dropped.
    let _telemetry = start_telemetry(&state, &hardware_sources, nvidia_gpus, amd_gpus);
    // start_heartbeat_loop (M1-45) goes here.
    serve_node_services(state).await
}

/// The state of a node with this identity and hardware, before any session or sample.
pub(crate) fn build_node_state(options: NodeOptions, identity: NodeIdentity, hardware: HardwareProfile) -> NodeState {
    let telemetry = NodeTelemetry {
        ram_total_bytes: hardware.ram_total_bytes,
        ..NodeTelemetry::default()
    };
    NodeState {
        node_id: rastai_core::ids::node_id_from_public_key(&public_key_bytes(&identity)),
        identity,
        mode: options.mode,
        paths: options.paths,
        settings: Mutex::new(options.settings),
        hardware: Mutex::new(hardware),
        telemetry: Mutex::new(telemetry),
        handshakes: Mutex::new(HandshakeTable::default()),
    }
}

/// A copy of the hardware profile.
pub(crate) fn current_hardware(state: &NodeState) -> HardwareProfile {
    // A panic while a lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state
        .hardware
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// A copy of the latest telemetry.
pub(crate) fn current_telemetry(state: &NodeState) -> NodeTelemetry {
    state
        .telemetry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// NodeService on node.sock. bind_local_socket refuses a socket folder another user owns or can enter, and the
/// server accepts only connections from this user (rastai-transport).
async fn serve_node_services(state: Arc<NodeState>) -> Result<(), NodeError> {
    let listener = bind_local_socket(&node_socket_path(&state.paths))?;
    let router = Server::builder().add_service(NodeServiceServer::new(NodeServiceHandler::new(state)));
    serve_grpc_on_socket(listener, router).await?;
    Ok(())
}

/// One node agent per user per machine (designer flow C1): two would lease the same GPUs. A node already answering
/// on node.sock stops this start before it touches anything, with that node's process id (RRS-N0003). Anything
/// else (no socket, a stale one) is left for bind_local_socket to settle.
#[cfg(unix)]
async fn refuse_second_node(options: &NodeOptions) -> Result<(), NodeError> {
    use rastai_transport::local_socket::{LocalStream, connect_local_socket};

    match connect_local_socket(&node_socket_path(&options.paths)).await {
        Ok(LocalStream::Unix(stream)) => {
            let process_id = stream.peer_cred().ok().and_then(|credentials| credentials.pid());
            Err(NodeError::NodeAlreadyRunning { process_id })
        }
        Err(_) => Ok(()),
    }
}

/// Local sockets come with named pipes on Windows (tier 2); bind_local_socket reports that.
#[cfg(not(unix))]
async fn refuse_second_node(_options: &NodeOptions) -> Result<(), NodeError> {
    Ok(())
}

fn log_hardware_notes(detected: &DetectedHardware) {
    for note in &detected.notes {
        log_diagnostic(note, &[]);
    }
}
