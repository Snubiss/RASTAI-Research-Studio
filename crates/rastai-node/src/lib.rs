//! rastai-node: the node agent (layout: rust-node-gateway-proto.md section 7). It runs embedded in the daemon and
//! as `rastai node serve`. Read node.rs first: `run_node` lists the start-up order.

pub mod error;
pub mod handshake;
pub mod hardware;
pub mod heartbeat;
pub mod hub_client;
pub mod identity;
pub mod jobs;
pub mod live;
pub mod node;
pub mod node_service;
pub mod permissions;
pub mod run_events;
pub mod telemetry;
pub mod worker_host_service;
