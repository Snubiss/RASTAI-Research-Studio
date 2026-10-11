//! rastai-transport: the byte pipes of the backend (layout: rust-node-gateway-proto.md section 4). M1-41 lands
//! local sockets with peer checks, gRPC channels and servers over them, the local NodeConnection and the plain-HTTP
//! half of the HTTP client. It decides nothing about what flows through them.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod error;
pub mod grpc_channel;
pub mod http_client;
pub mod local_socket;
pub mod node_connection;
pub mod peer;
