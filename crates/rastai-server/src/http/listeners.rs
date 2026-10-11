//! The daemon's two listeners (contracts 3.3, flows/platform.md 2.1): loopback TCP on 47820, falling back to the
//! next free port up to 47829, and daemon.sock. One router serves both. Every accepted TCP connection gets
//! TCP_NODELAY: without it, streaming small chunks stalled ~40 ms per reply in both bake-off gateways (spike 07).
//!
//! The chosen port goes into `<runtime>/daemon.port` only after both listeners are bound, so a client that reads it
//! finds a daemon that answers. A clean stop removes daemon.port and daemon.sock again (shutdown.rs).

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::serve::ListenerExt;
use rastai_core::paths::{DataPaths, daemon_port_path, daemon_socket_path};
use rastai_diag::logger::log_warn;
use rastai_transport::local_socket::{LocalListener, bind_local_socket};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::error::ServerError;
use crate::http::router::build_router;
use crate::server_state::Server;

/// The port range of contracts 3.3: the first free one is used, and written to daemon.port.
pub const FIRST_DEFAULT_PORT: u16 = 47820;
pub const LAST_DEFAULT_PORT: u16 = 47829;
/// Nothing listens beyond loopback unless `daemon.listen` says so (layout section 8.5).
pub const LOOPBACK_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// Accept errors other than a refused peer (too many open files) are retried after a pause, as axum does for
/// TCP, so a full file table doesn't spin the accept loop.
#[cfg(unix)]
const ACCEPT_ERROR_PAUSE_MS: u64 = 1000;

/// A daemon.sock connection's peer, as the kernel reported it. The router reads it to tell daemon.sock requests
/// from TCP ones (router.rs `listener_of`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketPeer {
    pub user_id: u32,
}

/// The bound listeners, serving.
#[derive(Debug)]
pub struct HttpListeners {
    pub tcp_port: u16,
    /// One task per listener; each ends once the server's shutdown flag is set and its open requests are done.
    pub serve_tasks: Vec<JoinHandle<()>>,
}

/// Binds TCP (the preferred port, then 47820–47829) and daemon.sock, writes daemon.port, and starts serving both.
pub async fn listen_http(
    server: &Arc<Server>,
    preferred_port: u16,
    listen_address: IpAddr,
) -> Result<HttpListeners, ServerError> {
    let tcp_listener = bind_tcp(listen_address, &candidate_ports(preferred_port)).await?;
    let tcp_port = bound_port(&tcp_listener)?;
    let socket_listener = bind_local_socket(&daemon_socket_path(&server.paths))?;
    write_daemon_port(&server.paths, tcp_port)?;
    let router = build_router(server.clone());
    let serve_tasks = vec![
        serve_tcp(tcp_listener, router.clone(), server.shutdown.subscribe()),
        serve_daemon_socket(socket_listener, router, server.shutdown.subscribe()),
    ];
    Ok(HttpListeners { tcp_port, serve_tasks })
}

/// The preferred port first, then the rest of the default range in order.
pub fn candidate_ports(preferred_port: u16) -> Vec<u16> {
    let mut ports = vec![preferred_port];
    for port in FIRST_DEFAULT_PORT..=LAST_DEFAULT_PORT {
        if port != preferred_port {
            ports.push(port);
        }
    }
    ports
}

/// The first of `ports` that binds on `address`, or `RRS-C1001` when every one is taken. Only a port in use moves on
/// to the next one: any other bind error (an address this machine doesn't have, a port this user may not open) would
/// fail the same way on every port, so it stops the search and is reported with its own reason.
pub async fn bind_tcp(address: IpAddr, ports: &[u16]) -> Result<TcpListener, ServerError> {
    for port in ports {
        let socket_address = SocketAddr::new(address, *port);
        match TcpListener::bind(socket_address).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(listen_address_error(socket_address, &error)),
        }
    }
    Err(ServerError::NoFreePort {
        tried_ports: ports.to_vec(),
    })
}

fn listen_address_error(address: SocketAddr, error: &io::Error) -> ServerError {
    ServerError::ListenAddressUnusable {
        address,
        reason: error.to_string(),
    }
}

/// The port actually bound: the preferred one may be 0 (the operating system picks), which tests use.
fn bound_port(listener: &TcpListener) -> Result<u16, ServerError> {
    match listener.local_addr() {
        Ok(address) => Ok(address.port()),
        Err(_) => Err(ServerError::NoFreePort {
            tried_ports: Vec::new(),
        }),
    }
}

/// `<runtime>/daemon.port`: the port as decimal digits and a newline.
fn write_daemon_port(paths: &DataPaths, port: u16) -> Result<(), ServerError> {
    let path = daemon_port_path(paths);
    std::fs::write(&path, format!("{port}\n")).map_err(|error| runtime_file_error(&path, error))
}

/// Removes daemon.port and daemon.sock after a clean stop. Either may already be gone; that's fine.
pub fn remove_runtime_files(paths: &DataPaths) {
    let _ = std::fs::remove_file(daemon_port_path(paths));
    let _ = std::fs::remove_file(daemon_socket_path(paths));
}

fn runtime_file_error(path: &Path, error: io::Error) -> ServerError {
    ServerError::RuntimeFileNotWritable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

fn serve_tcp(listener: TcpListener, router: Router, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    let listener = listener.tap_io(set_no_delay);
    tokio::spawn(async move {
        let serving = axum::serve(listener, router).with_graceful_shutdown(stop_requested(stop));
        if let Err(error) = serving.await {
            log_warn("the TCP listener stopped", &[("reason", &error.to_string())]);
        }
    })
}

fn set_no_delay(stream: &mut TcpStream) {
    if let Err(error) = stream.set_nodelay(true) {
        log_warn("TCP_NODELAY could not be set", &[("reason", &error.to_string())]);
    }
}

#[cfg(unix)]
fn serve_daemon_socket(listener: LocalListener, router: Router, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    let listener = DaemonSocketListener { listener };
    let service = router.into_make_service_with_connect_info::<SocketPeer>();
    tokio::spawn(async move {
        let serving = axum::serve(listener, service).with_graceful_shutdown(stop_requested(stop));
        if let Err(error) = serving.await {
            log_warn("the daemon.sock listener stopped", &[("reason", &error.to_string())]);
        }
    })
}

/// No `LocalListener` can exist off Unix yet (rastai-transport binds named pipes with tier 2), so this is never called.
#[cfg(not(unix))]
fn serve_daemon_socket(listener: LocalListener, _router: Router, _stop: watch::Receiver<bool>) -> JoinHandle<()> {
    match listener {}
}

/// Resolves once the shutdown flag is set, or once nobody can set it any more.
async fn stop_requested(mut stop: watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopping| *stopping).await;
}

/// daemon.sock as an axum listener. `accept_local` refuses a peer of another user and closes its connection, so
/// every connection served here is the owner's.
#[cfg(unix)]
struct DaemonSocketListener {
    listener: LocalListener,
}

#[cfg(unix)]
impl axum::serve::Listener for DaemonSocketListener {
    type Io = tokio::net::UnixStream;
    type Addr = SocketPeer;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        use std::time::Duration;

        use rastai_transport::error::TransportError;
        use rastai_transport::local_socket::{LocalStream, accept_local};

        loop {
            match accept_local(&self.listener).await {
                Ok((LocalStream::Unix(stream), peer)) => return (stream, SocketPeer { user_id: peer.user_id }),
                Err(TransportError::PeerIsAnotherUser { peer_user_id, .. }) => {
                    log_warn(
                        "refused a daemon.sock connection from another user",
                        &[("peer_user_id", &peer_user_id.to_string())],
                    );
                }
                Err(error) => {
                    log_warn("accepting on daemon.sock failed", &[("code", error.code())]);
                    tokio::time::sleep(Duration::from_millis(ACCEPT_ERROR_PAUSE_MS)).await;
                }
            }
        }
    }

    /// A local socket's address is its path, which the caller already has; axum never asks while serving.
    fn local_addr(&self) -> io::Result<Self::Addr> {
        Err(io::Error::other("daemon.sock has no peer of its own"))
    }
}

#[cfg(unix)]
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, DaemonSocketListener>> for SocketPeer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, DaemonSocketListener>) -> SocketPeer {
        *stream.remote_addr()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// TEST-NET-1 (RFC 5737): reserved for documentation, so no test machine has it.
    const ADDRESS_NOT_ON_THIS_MACHINE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

    #[tokio::test]
    async fn an_address_this_machine_lacks_is_reported_as_such_not_as_no_free_port() {
        let result = bind_tcp(ADDRESS_NOT_ON_THIS_MACHINE, &[47820, 47821]).await;
        let refused = result.err().unwrap();
        assert_eq!(refused.code(), "RRS-C0002");
        match refused {
            ServerError::ListenAddressUnusable { address, reason } => {
                // The search stopped at the first port: the next one would fail the same way.
                assert_eq!(address, SocketAddr::new(ADDRESS_NOT_ON_THIS_MACHINE, 47820));
                assert!(!reason.is_empty());
            }
            other => panic!("expected ListenAddressUnusable, got {other:?}"),
        }
    }

    #[test]
    fn the_preferred_port_comes_first_then_the_range() {
        assert_eq!(
            candidate_ports(47820),
            vec![47820, 47821, 47822, 47823, 47824, 47825, 47826, 47827, 47828, 47829]
        );
        let custom = candidate_ports(8899);
        assert_eq!(custom.len(), 11);
        assert_eq!(custom[0], 8899);
        assert_eq!(custom[1], 47820);
        let inside = candidate_ports(47825);
        assert_eq!(inside[0], 47825);
        assert_eq!(inside.len(), 10);
    }
}
