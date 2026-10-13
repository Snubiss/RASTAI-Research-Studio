//! A clean stop (flows/platform.md 2.1): SIGTERM, SIGINT or `system.shutdown` set the server's shutdown flag; the
//! listeners stop taking requests and finish the open ones, daemon.port and daemon.sock are removed, the store's
//! writer commits what is queued, and the daemon lock is released last, so a new daemon can't start while this one
//! still writes. Stopping the daemon never stops jobs; `job.cancel` does. Open `Subscribe` streams, run tails, the
//! scheduler, the node links' watch and the node hub end on the shutdown flag. The embedded node is stopped next, so
//! nothing calls the hub any more; its workers run on in their own process groups and register again with the next
//! node (flows/platform.md 2.3).
//!
//! The store is closed once nothing else holds the server: a run tail or a settle that was mid-write when the flag
//! went up gets `SHUTDOWN_GRACE_SECONDS` to let it go. Aborting a listener after the grace period doesn't end the
//! connection tasks axum spawned for it, and those still hold the server, so its store can't be closed. Then the lock
//! is kept until the process exits (`run_daemon` returns straight after), never released while a handler may still
//! write. Streams end on the flag and let the server go before their last envelope, so this path stays rare.

use std::sync::Arc;
use std::time::Duration;

use rastai_core::paths::node_hub_socket_path;
use rastai_diag::logger::{log_info, log_warn};
use rastai_store::data_dir_lock::DaemonLock;
use rastai_store::store::close_store;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::daemon::{RunningDaemon, stop_embedded_node};
use crate::error::ServerError;
use crate::http::listeners::remove_runtime_files;
use crate::server_state::Server;

/// How long open requests get to finish once a stop is asked for; after that their tasks are dropped, so a client
/// that never hangs up can't keep the daemon (and its lock) alive.
const SHUTDOWN_GRACE_SECONDS: u64 = 10;
/// How often the stop looks whether the last task let the server go.
const SERVER_RELEASE_POLL_MS: u64 = 20;

/// Asks the daemon to stop. Safe to call any number of times, from anywhere.
pub fn request_shutdown(server: &Server) {
    // send_replace sets the flag even when no listener is watching yet; `send` would drop it.
    server.shutdown.send_replace(true);
}

/// SIGTERM and SIGINT stop the daemon. Only `run_daemon` installs this; tests stop a daemon with `request_shutdown`
/// and keep their process's own signal handling.
pub fn stop_on_signals(server: &Server) {
    let shutdown = server.shutdown.clone();
    tokio::spawn(async move {
        wait_for_stop_signal().await;
        log_info("stop signal received", &[]);
        shutdown.send_replace(true);
    });
}

/// Waits until a stop is asked for, then stops everything `start_daemon` started, in the order the header gives.
pub async fn wait_for_shutdown(running: RunningDaemon) -> Result<(), ServerError> {
    let RunningDaemon {
        server,
        http_port: _,
        serve_tasks,
        daemon_lock,
        embedded_node,
    } = running;
    wait_for_flag(server.shutdown.subscribe()).await;
    finish_serving(serve_tasks).await;
    stop_embedded_node(embedded_node).await;
    remove_runtime_files(&server.paths);
    let _ = std::fs::remove_file(node_hub_socket_path(&server.paths));
    let store_closed = close_server(server).await;
    if store_closed {
        drop(daemon_lock);
    } else {
        keep_lock_until_exit(daemon_lock);
    }
    log_info("daemon stopped", &[]);
    Ok(())
}

async fn wait_for_flag(mut flag: watch::Receiver<bool>) {
    wait_until_stopping(&mut flag).await;
}

/// Returns once a stop is asked for: what streams and run tails wait on beside their work. It holds nothing across
/// its own return, so a task that selects on it stays `Send`.
pub async fn wait_until_stopping(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stopping| *stopping).await;
}

/// Waits for the listeners to finish their open requests, up to the grace period, then drops whatever is left.
async fn finish_serving(serve_tasks: Vec<JoinHandle<()>>) {
    let grace = Duration::from_secs(SHUTDOWN_GRACE_SECONDS);
    for task in serve_tasks {
        let abort = task.abort_handle();
        if tokio::time::timeout(grace, task).await.is_err() {
            log_warn(
                "a listener was still serving after the grace period; its requests were dropped",
                &[],
            );
            abort.abort();
        }
    }
}

/// Closes the store once nothing else holds the server, waiting up to the grace period for the tasks that still do,
/// and says whether it did. A handler task still holding the server after that keeps the writer alive; then the
/// writer finishes its queue on its own when that task ends.
async fn close_server(server: Arc<Server>) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(SHUTDOWN_GRACE_SECONDS);
    let mut server = server;
    loop {
        match Arc::try_unwrap(server) {
            Ok(server) => {
                close_store(server.store).await;
                return true;
            }
            Err(still_shared) if tokio::time::Instant::now() < deadline => {
                server = still_shared;
                tokio::time::sleep(Duration::from_millis(SERVER_RELEASE_POLL_MS)).await;
            }
            Err(_still_shared) => {
                log_warn(
                    "the server was still in use at shutdown; the store stays open and the daemon lock is kept until exit",
                    &[],
                );
                return false;
            }
        }
    }
}

/// Never releases the lock in this process: a handler still holding the store may write, and a new daemon must not
/// start on the same data until it can't. The operating system releases the lock when the process exits.
fn keep_lock_until_exit(daemon_lock: DaemonLock) {
    std::mem::forget(daemon_lock);
}

#[cfg(unix)]
async fn wait_for_stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(error) => {
            log_warn(
                "SIGTERM can't be watched; only SIGINT stops the daemon",
                &[("reason", &error.to_string())],
            );
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
