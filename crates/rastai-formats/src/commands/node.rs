//! The `node` area's types (contracts 6.4, 9.2): `node.list` and `node.status`, and `NodeSummary`, one row of the
//! `nodes` table, which `system.status` returns too.
//!
//! Node states are the proto enum's names (contracts 9.2), written as text, for the reason job.rs gives.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::job::JobSummary;
use crate::commands::{command_type, positional};
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

const LIST_COMMAND_ID: &str = "node.list";
const STATUS_COMMAND_ID: &str = "node.status";
const NODE_SUMMARY_TYPE: &str = "NodeSummary";
/// The telemetry of the node's last heartbeat; `node.heartbeat`'s payload is this message (contracts 7.3).
const NODE_TELEMETRY_MESSAGE: &str = "rastai.common.v1.NodeTelemetry";

/// One node, as the `nodes` table holds it (contracts 8.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub node_id: String,
    pub name: String,
    /// `local` for the embedded node, `ssh` for a paired one.
    pub transport: String,
    /// Empty for the embedded node; `user@host` for an SSH node.
    #[serde(default)]
    pub address: String,
    /// A node connection state of contracts 9.2, such as `connected`.
    pub state: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    /// 0 before the first heartbeat.
    #[serde(default)]
    pub last_heartbeat_unix_ms: u64,
}

/// `node.list` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeListArgs {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeListResult {
    pub nodes: Vec<NodeSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatusArgs {
    pub node_id: String,
}

/// One node in detail: its row, its last telemetry and its jobs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatusResult {
    pub node: NodeSummary,
    /// The last heartbeat's `NodeTelemetry` as protobuf JSON. Left out before the first heartbeat, because a
    /// field is never `null`.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub telemetry: Value,
    pub jobs: Vec<JobSummary>,
}

pub fn describe_node_summary() -> TypeDescription {
    let fields = vec![
        field("node_id", FieldKind::String, REQUIRED, "The node's id."),
        field("name", FieldKind::String, REQUIRED, "The node's name."),
        field(
            "transport",
            FieldKind::String,
            REQUIRED,
            "local for the embedded node, ssh for a paired one.",
        ),
        field(
            "address",
            FieldKind::String,
            OPTIONAL,
            "user@host for an SSH node; empty for the embedded one.",
        ),
        field(
            "state",
            FieldKind::String,
            REQUIRED,
            "Its connection state, such as connected.",
        ),
        field(
            "protocol_version",
            FieldKind::Integer,
            REQUIRED,
            "The node protocol version it speaks.",
        ),
        field(
            "capabilities",
            list_of(FieldKind::String),
            REQUIRED,
            "The capability flags of its handshake.",
        ),
        field(
            "last_heartbeat_unix_ms",
            FieldKind::Integer,
            OPTIONAL,
            "When its last heartbeat arrived; 0 or absent before the first.",
        ),
    ];
    command_type(
        NODE_SUMMARY_TYPE,
        LIST_COMMAND_ID,
        "One node: how the daemon reaches it, its state and its last heartbeat.",
        fields,
    )
}

pub fn describe_node_list_args() -> TypeDescription {
    command_type(
        "NodeListArgs",
        LIST_COMMAND_ID,
        "node.list takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_node_list_result() -> TypeDescription {
    let fields = vec![field(
        "nodes",
        list_of(FieldKind::Type(NODE_SUMMARY_TYPE.to_string())),
        REQUIRED,
        "Every known node, the embedded one first.",
    )];
    command_type("NodeListResult", LIST_COMMAND_ID, "The known nodes.", fields)
}

pub fn describe_node_status_args() -> TypeDescription {
    let fields = vec![positional(
        0,
        field("node_id", FieldKind::String, REQUIRED, "The node to show."),
    )];
    command_type("NodeStatusArgs", STATUS_COMMAND_ID, "Which node to show.", fields)
}

pub fn describe_node_status_result() -> TypeDescription {
    let fields = vec![
        field(
            "node",
            FieldKind::Type(NODE_SUMMARY_TYPE.to_string()),
            REQUIRED,
            "The node.",
        ),
        field(
            "telemetry",
            FieldKind::Proto(NODE_TELEMETRY_MESSAGE.to_string()),
            OPTIONAL,
            "Its last heartbeat's telemetry; absent before the first heartbeat.",
        ),
        field(
            "jobs",
            list_of(FieldKind::Type("JobSummary".to_string())),
            REQUIRED,
            "The jobs on the node that haven't finished.",
        ),
    ];
    command_type(
        "NodeStatusResult",
        STATUS_COMMAND_ID,
        "One node in detail: its state, last telemetry and unfinished jobs.",
        fields,
    )
}
