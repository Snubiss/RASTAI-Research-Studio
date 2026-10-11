//! The `system` area's types (contracts 6.4): `system.status` and `system.shutdown`.

use serde::{Deserialize, Serialize};

use crate::commands::command_type;
use crate::commands::node::NodeSummary;
use crate::describe::types::{FieldKind, REQUIRED, TypeDescription, field, list_of};

const STATUS_COMMAND_ID: &str = "system.status";
const SHUTDOWN_COMMAND_ID: &str = "system.shutdown";

/// `system.status` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemStatusArgs {}

/// The daemon's versions, the protocol versions it speaks, and its nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemStatusResult {
    pub app_version: String,
    pub plugin_api_version: String,
    pub node_protocol_version: u32,
    pub worker_protocol_version: u32,
    /// The wire packages the daemon serves, such as `rastai.commands.v1` (layout 11.1).
    pub api_packages: Vec<String>,
    pub nodes: Vec<NodeSummary>,
}

/// `system.shutdown` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemShutdownArgs {}

/// What the daemon left behind when it stopped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemShutdownResult {
    /// Jobs that keep running in their own process groups; the next daemon start reattaches them.
    pub running_job_ids: Vec<String>,
}

pub fn describe_system_status_args() -> TypeDescription {
    command_type(
        "SystemStatusArgs",
        STATUS_COMMAND_ID,
        "system.status takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_system_status_result() -> TypeDescription {
    let fields = vec![
        field("app_version", FieldKind::String, REQUIRED, "The daemon's app version."),
        field(
            "plugin_api_version",
            FieldKind::String,
            REQUIRED,
            "The plugin API version plugins are matched against.",
        ),
        field(
            "node_protocol_version",
            FieldKind::Integer,
            REQUIRED,
            "The node protocol version the daemon speaks.",
        ),
        field(
            "worker_protocol_version",
            FieldKind::Integer,
            REQUIRED,
            "The worker protocol version of this release.",
        ),
        field(
            "api_packages",
            list_of(FieldKind::String),
            REQUIRED,
            "The wire packages the daemon serves, such as rastai.commands.v1.",
        ),
        field(
            "nodes",
            list_of(FieldKind::Type("NodeSummary".to_string())),
            REQUIRED,
            "Every known node, the embedded one first.",
        ),
    ];
    command_type(
        "SystemStatusResult",
        STATUS_COMMAND_ID,
        "The daemon's versions, the protocol versions it speaks, and its nodes.",
        fields,
    )
}

pub fn describe_system_shutdown_args() -> TypeDescription {
    command_type(
        "SystemShutdownArgs",
        SHUTDOWN_COMMAND_ID,
        "system.shutdown takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_system_shutdown_result() -> TypeDescription {
    let fields = vec![field(
        "running_job_ids",
        list_of(FieldKind::String),
        REQUIRED,
        "Jobs that keep running in their own process groups; the next daemon start reattaches them.",
    )];
    command_type(
        "SystemShutdownResult",
        SHUTDOWN_COMMAND_ID,
        "What the daemon left running when it stopped.",
        fields,
    )
}
