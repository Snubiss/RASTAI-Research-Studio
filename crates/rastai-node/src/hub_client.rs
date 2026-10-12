//! The node's client of the daemon's `NodeHubService` (contracts 3.2 and 4.4; layout: rust-node-gateway-proto.md
//! sections 7.1 and 7.9). The daemon names the hub socket in its handshake (`HandshakeRequest.hub_socket_path`,
//! signed with its key): `node-hub.sock` for the embedded node, the reverse-forwarded socket for a remote one. The
//! node dials it only once a daemon has handshaken, and keeps one channel to it in `NodeState.hub`.
//!
//! In M1 the client makes one call, `Heartbeat` (heartbeat.rs decides when). `ExecuteCommand` joins with plugin
//! modules (M4), `GatewayGenerate` and `GetSecret` with the milestones that use them.
//!
//! The session token isn't sent yet: the gRPC metadata key it travels under is a wire name, which lands in a contract
//! PR with the daemon's node link (layout 7.9, "Not checked yet"). Until then the hub socket is the daemon's own,
//! reached only by the same user.

use std::path::Path;
use std::sync::{MutexGuard, PoisonError};
use std::time::Duration;

use rastai_proto::node::v1::HeartbeatRequest;
use rastai_proto::node::v1::node_hub_service_client::NodeHubServiceClient;
use rastai_transport::grpc_channel::connect_grpc_channel;
use tonic::transport::Channel;

use crate::handshake::current_session;
use crate::node::NodeState;

/// How long one hub call may take. A daemon that doesn't answer in this time has missed the beat; the next tick
/// tries again, so a hung daemon never holds the heartbeat loop.
const HUB_CALL_TIMEOUT_SECONDS: u64 = 5;

/// The node's channel to the daemon's `NodeHubService`, and the socket it was opened on.
#[derive(Debug, Clone)]
pub struct HubConnection {
    pub hub_socket_path: String,
    pub channel: Channel,
}

/// Sends one heartbeat to the daemon of the current session. With no session yet there is no daemon to tell, and
/// nothing is sent. The error says why the beat didn't arrive; the channel is then dropped, so the next beat dials
/// again (a restarted daemon serves a new socket).
pub async fn send_heartbeat(state: &NodeState, request: HeartbeatRequest) -> Result<(), String> {
    let Some(session) = current_session(state) else {
        return Ok(());
    };
    let channel = hub_channel(state, &session.hub_socket_path).await?;
    let mut client = NodeHubServiceClient::new(channel);
    let answered = tokio::time::timeout(hub_call_timeout(), client.heartbeat(request)).await;
    let failure = match answered {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(status)) => format!("the Heartbeat call failed: {}", status.message()),
        Err(_) => format!("the daemon didn't answer the Heartbeat within {HUB_CALL_TIMEOUT_SECONDS} seconds"),
    };
    forget_hub_connection(state);
    Err(failure)
}

/// The channel to `hub_socket_path`: the one kept, if it goes there, or a new one, which is kept.
async fn hub_channel(state: &NodeState, hub_socket_path: &str) -> Result<Channel, String> {
    if let Some(channel) = kept_channel(state, hub_socket_path) {
        return Ok(channel);
    }
    let connecting = connect_grpc_channel(Path::new(hub_socket_path));
    let channel = match tokio::time::timeout(hub_call_timeout(), connecting).await {
        Ok(Ok(channel)) => channel,
        Ok(Err(error)) => return Err(format!("the hub socket {hub_socket_path} can't be reached: {error:?}")),
        Err(_) => return Err(format!("the hub socket {hub_socket_path} didn't answer")),
    };
    *lock_hub(state) = Some(HubConnection {
        hub_socket_path: hub_socket_path.to_string(),
        channel: channel.clone(),
    });
    Ok(channel)
}

fn hub_call_timeout() -> Duration {
    Duration::from_secs(HUB_CALL_TIMEOUT_SECONDS)
}

fn kept_channel(state: &NodeState, hub_socket_path: &str) -> Option<Channel> {
    let hub = lock_hub(state);
    let connection = hub.as_ref()?;
    if connection.hub_socket_path != hub_socket_path {
        return None;
    }
    Some(connection.channel.clone())
}

fn forget_hub_connection(state: &NodeState) {
    *lock_hub(state) = None;
}

fn lock_hub(state: &NodeState) -> MutexGuard<'_, Option<HubConnection>> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state.hub.lock().unwrap_or_else(PoisonError::into_inner)
}
