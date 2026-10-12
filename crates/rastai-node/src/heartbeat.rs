//! Heartbeats (layout: rust-node-gateway-proto.md sections 7.5 and 7.9; contracts 7.3 and 9.2): a snapshot of the
//! node's state sent to the daemon every `nodes.heartbeat_seconds` (5), and at once on any change the daemon must see.
//!
//! The node's own durable state (job states today; lease, model and sandbox states as M3 and M5 add them) is in no
//! event log. The daemon learns it only from heartbeats and turns the differences into `job.state_changed` and the
//! like, so a change must not wait for the next tick. Whoever changes such state calls `request_heartbeat`, which only
//! wakes the loop; the loop sends at most one beat per `HEARTBEAT_COALESCE_MS`, so a burst of changes is one beat,
//! and the changes that come during the pause ride on the beat right after it.
//!
//! A heartbeat that fails is not retried at once: the next tick sends a fresh snapshot. The daemon counts missed
//! beats itself (three mark the node unreachable, contracts 9.2), and jobs never stop because the daemon is gone.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use rastai_proto::node::v1::HeartbeatRequest;
use tokio::task::JoinHandle;

use crate::hub_client::send_heartbeat;
use crate::jobs::supervisor::list_jobs;
use crate::node::{NodeState, current_telemetry};

/// The shortest gap between two beats (layout 7.5: "coalesced to one per 100 ms").
const HEARTBEAT_COALESCE_MS: u64 = 100;
/// A `nodes.heartbeat_seconds` of 0 would beat without pause, so the loop waits at least this long between ticks.
const MIN_HEARTBEAT_SECONDS: u64 = 1;

/// The running heartbeat loop. It stops when this is dropped, like the telemetry loops.
pub struct HeartbeatLoop {
    handle: JoinHandle<()>,
}

impl Drop for HeartbeatLoop {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Starts the loop that sends a heartbeat every `nodes.heartbeat_seconds` and whenever `request_heartbeat` asks.
pub fn start_heartbeat_loop(state: &Arc<NodeState>) -> HeartbeatLoop {
    let handle = tokio::spawn(run_heartbeats(Arc::clone(state)));
    HeartbeatLoop { handle }
}

/// Asks for a heartbeat now, after a change the daemon must see. It never waits, so it can be called anywhere a
/// state changes; several calls before the loop wakes give one beat.
pub fn request_heartbeat(state: &NodeState) {
    state.heartbeat_now.notify_one();
}

/// The node's state as one snapshot: its id, the latest telemetry and every job it knows.
pub fn build_heartbeat(state: &NodeState) -> HeartbeatRequest {
    HeartbeatRequest {
        node_id: state.node_id.clone(),
        telemetry: Some(current_telemetry(state)),
        jobs: list_jobs(state),
    }
}

async fn run_heartbeats(state: Arc<NodeState>) {
    loop {
        wait_for_next_beat(&state).await;
        // A failed beat is the daemon's to count (contracts 9.2); the next tick sends a fresh one.
        let _ = send_heartbeat(&state, build_heartbeat(&state)).await;
        tokio::time::sleep(Duration::from_millis(HEARTBEAT_COALESCE_MS)).await;
    }
}

/// Returns at the next tick, or as soon as a heartbeat is asked for (at once if one was asked for meanwhile).
async fn wait_for_next_beat(state: &NodeState) {
    let interval = Duration::from_secs(heartbeat_seconds(state));
    tokio::select! {
        _ = tokio::time::sleep(interval) => {}
        _ = state.heartbeat_now.notified() => {}
    }
}

/// `nodes.heartbeat_seconds`, read at each tick so a settings change takes effect without a restart.
fn heartbeat_seconds(state: &NodeState) -> u64 {
    let settings = state.settings.lock().unwrap_or_else(PoisonError::into_inner);
    settings.heartbeat_seconds.max(MIN_HEARTBEAT_SECONDS)
}
