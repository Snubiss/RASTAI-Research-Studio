//! The daemon's way to a node (layout: rust-node-gateway-proto.md section 4.2): a `NodeTarget` says where the node
//! is, `open_node_connection` opens the channel, and `node_channel` hands it to a `NodeServiceClient`.
//!
//! M1 has the local target only: the embedded node on node.sock (build-order step 1). The `Ssh` target and
//! connection, which run an OpenSSH child with -L and -R forwards (section 4.3), arrive with ssh.rs in step 9.

use std::path::PathBuf;

use rastai_core::paths::DataPaths;
use tonic::transport::Channel;

use crate::error::TransportError;
use crate::grpc_channel::connect_grpc_channel;

/// Where a node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTarget {
    /// A node on this machine: its node.sock (`rastai_core::paths::node_socket_path`).
    Local { socket_path: PathBuf },
}

/// An open connection to a node.
#[derive(Debug, Clone)]
pub enum NodeConnection {
    Local { channel: Channel },
}

/// Opens the gRPC channel to the node at `target`. RRS-N2004 when nothing listens there.
/// `_paths` is where the Ssh target (step 9) puts its forwarded socket; the local target needs nothing from it.
pub async fn open_node_connection(target: &NodeTarget, _paths: &DataPaths) -> Result<NodeConnection, TransportError> {
    match target {
        NodeTarget::Local { socket_path } => {
            let channel = connect_grpc_channel(socket_path).await?;
            Ok(NodeConnection::Local { channel })
        }
    }
}

/// The channel for `NodeServiceClient::new`. Cloning a tonic channel is cheap: the copies share one connection.
pub fn node_channel(connection: &NodeConnection) -> Channel {
    match connection {
        NodeConnection::Local { channel } => channel.clone(),
    }
}

/// Closes the connection. A local one closes when its last channel copy is dropped; the Ssh one (step 9) will
/// also stop its OpenSSH child here.
pub async fn close_node_connection(connection: NodeConnection) {
    match connection {
        NodeConnection::Local { channel } => drop(channel),
    }
}
