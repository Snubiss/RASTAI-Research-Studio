//! tonic channels and servers over local sockets (layout: rust-node-gateway-proto.md section 4.2). The node serves
//! NodeService on node.sock and WorkerHostService on worker-host.sock with `serve_grpc_on_socket`, and dials the job
//! sockets and node-hub.sock with `connect_grpc_channel`.
//!
//! The client uses tonic's own Unix-socket endpoint (`unix:<path>`), so this file needs no tower connector. Before
//! it connects it checks the socket's folder, as `connect_local_socket` does: only this user can bind a socket in a
//! 0700 folder this user owns. The server accepts through `accept_local`, so a peer of another user never reaches
//! a service.

use std::path::Path;

use tonic::transport::Channel;
use tonic::transport::server::Router;

use crate::error::TransportError;
use crate::local_socket::LocalListener;
#[cfg(unix)]
use crate::local_socket::{LocalStream, accept_local, check_folder_before_connecting};

/// tonic's prefix for an endpoint that is a Unix socket path.
#[cfg(unix)]
const UNIX_ENDPOINT_PREFIX: &str = "unix:";
/// Accepted connections that may wait for tonic to take them. Local clients are few (the daemon, a worker), so a
/// short queue is plenty; a full one only makes the accept loop wait.
#[cfg(unix)]
const ACCEPT_QUEUE_LENGTH: usize = 16;
/// After a failed accept (out of file descriptors, say) the loop waits this long, so it can't spin.
#[cfg(unix)]
const ACCEPT_RETRY_DELAY_MS: u64 = 100;

/// The accepted connections tonic reads from: plain Unix streams, whose peers already passed the same-user check.
#[cfg(unix)]
type AcceptedSender = tokio::sync::mpsc::Sender<Result<tokio::net::UnixStream, std::io::Error>>;

/// A channel to the gRPC server on `socket_path`. Nothing listening is RRS-N2004; a folder another user owns or
/// can enter is RRS-N2005.
#[cfg(unix)]
pub async fn connect_grpc_channel(socket_path: &Path) -> Result<Channel, TransportError> {
    check_folder_before_connecting(socket_path)?;
    let Some(path_text) = socket_path.to_str() else {
        return Err(not_reachable(
            socket_path,
            "the path isn't UTF-8, which a gRPC endpoint needs",
        ));
    };
    let endpoint = tonic::transport::Endpoint::from_shared(format!("{UNIX_ENDPOINT_PREFIX}{path_text}"))
        .map_err(|error| not_reachable(socket_path, describe_error(&error)))?;
    endpoint
        .connect()
        .await
        .map_err(|error| not_reachable(socket_path, describe_error(&error)))
}

#[cfg(not(unix))]
pub async fn connect_grpc_channel(_socket_path: &Path) -> Result<Channel, TransportError> {
    Err(TransportError::NotSupportedOnThisPlatform {
        what: "connect_grpc_channel",
    })
}

/// Serves `router` on `listener` until the server stops. Each connection passes `accept_local` first, so a peer of
/// another user is logged and closed and never reaches a service.
#[cfg(unix)]
pub async fn serve_grpc_on_socket(listener: LocalListener, router: Router) -> Result<(), TransportError> {
    let (sender, receiver) = tokio::sync::mpsc::channel(ACCEPT_QUEUE_LENGTH);
    tokio::spawn(accept_same_user_connections(listener, sender));
    let incoming = tonic::codegen::tokio_stream::wrappers::ReceiverStream::new(receiver);
    router
        .serve_with_incoming(incoming)
        .await
        .map_err(|error| crate::error::connection_failed(describe_error(&error)))
}

#[cfg(not(unix))]
pub async fn serve_grpc_on_socket(listener: LocalListener, _router: Router) -> Result<(), TransportError> {
    // No LocalListener can exist here yet (see the enum in local_socket.rs), so there is nothing to match.
    match listener {}
}

/// The accept loop: hands each same-user connection to tonic, until tonic stops reading (the server ended).
#[cfg(unix)]
async fn accept_same_user_connections(listener: LocalListener, sender: AcceptedSender) {
    loop {
        // Waiting on `closed` as well means a stopped server releases the listener at once, not at the next
        // connection.
        let accepted = tokio::select! {
            accepted = accept_local(&listener) => accepted,
            () = sender.closed() => return,
        };
        match accepted {
            Ok((LocalStream::Unix(stream), _peer)) => {
                if sender.send(Ok(stream)).await.is_err() {
                    return;
                }
            }
            Err(error) => log_refused_connection(&error).await,
        }
    }
}

/// A refused peer or a failed accept never stops the server; it goes to the log.
#[cfg(unix)]
async fn log_refused_connection(error: &TransportError) {
    let reason = format!("{error:?}");
    rastai_diag::logger::log_warn(
        "local socket connection refused",
        &[("code", error.code()), ("reason", &reason)],
    );
    if !matches!(error, TransportError::PeerIsAnotherUser { .. }) {
        tokio::time::sleep(std::time::Duration::from_millis(ACCEPT_RETRY_DELAY_MS)).await;
    }
}

/// tonic's errors say "transport error" and keep the useful part in their source, so both go in the reason.
#[cfg(unix)]
fn describe_error(error: &dyn std::error::Error) -> String {
    match error.source() {
        Some(source) => format!("{error}: {source}"),
        None => error.to_string(),
    }
}

#[cfg(unix)]
fn not_reachable(socket_path: &Path, reason: impl std::fmt::Display) -> TransportError {
    TransportError::SocketNotReachable {
        socket_path: socket_path.to_path_buf(),
        reason: reason.to_string(),
    }
}
