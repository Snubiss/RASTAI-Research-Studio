//! rastai-node: the node agent (layout: rust-node-gateway-proto.md section 7). It runs embedded in the daemon and
//! as `rastai node serve`. Read node.rs first: `run_node` lists the start-up order.

pub mod downloads;
pub mod error;
pub mod handshake;
pub mod hardware;
pub mod heartbeat;
pub mod hf_cache;
pub mod hub_client;
pub mod identity;
pub mod jobs;
pub mod live;
pub mod node;
pub mod node_service;
pub mod pause_points;
pub mod permissions;
pub mod run_events;
pub mod telemetry;
#[cfg(all(test, unix))]
mod test_support;
pub mod worker_host_service;
