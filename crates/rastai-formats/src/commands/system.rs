//! The `system` area's types (contracts 6.4): `system.status` and `system.shutdown`, and from M2 the hardware,
//! environment, doctor and bundle commands.
//!
//! A command that names a node takes `node_id`; empty means the embedded node, so a CLI call needs no node flag.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::command_type;
use crate::commands::node::NodeSummary;
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};
use crate::documents::plans::EnvironmentInstallPlan;

const STATUS_COMMAND_ID: &str = "system.status";
const SHUTDOWN_COMMAND_ID: &str = "system.shutdown";
const HARDWARE_COMMAND_ID: &str = "system.hardware";
const ENV_LIST_COMMAND_ID: &str = "system.env.list";
const ENV_INSTALL_COMMAND_ID: &str = "system.env.install";
const DOCTOR_COMMAND_ID: &str = "system.doctor";
const BUNDLE_COMMAND_ID: &str = "system.bundle";
/// The wire messages a result carries as protobuf JSON (contracts 4, 4.9).
const HARDWARE_PROFILE_MESSAGE: &str = "rastai.common.v1.HardwareProfile";
const ENVIRONMENT_INFO_MESSAGE: &str = "rastai.node.v1.EnvironmentInfo";
const ENVIRONMENT_INSTALL_PLAN_TYPE: &str = "EnvironmentInstallPlan";
/// The backends a node can pick (node layout 7.6, contracts 4.1 `HardwareProfile.backend`).
const BACKENDS: &[&str] = &["cuda13", "cuda126", "rocm", "cpu"];

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

/// `system.hardware` names the node to read; empty means the embedded node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemHardwareArgs {
    #[serde(default)]
    pub node_id: String,
}

/// A node's hardware profile, with its last self-test verdicts (contracts 4.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemHardwareResult {
    pub node_id: String,
    /// The `HardwareProfile` as protobuf JSON (contracts 4).
    pub hardware: Value,
}

/// `system.env.list` names the node whose environments to list; empty means the embedded node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemEnvListArgs {
    #[serde(default)]
    pub node_id: String,
}

/// The node's uv environments with their lock hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemEnvListResult {
    /// Each `EnvironmentInfo` as protobuf JSON (contracts 4.9).
    pub environments: Vec<Value>,
}

/// `system.env.install` creates or syncs the environment of one backend on a node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemEnvInstallArgs {
    #[serde(default)]
    pub node_id: String,
    /// One of `BACKENDS`.
    pub backend: String,
}

/// What `system.env.install` answers: its plan. A dry run returns only this; a real run also starts the job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemEnvInstallResult {
    pub plan: EnvironmentInstallPlan,
}

/// `system.doctor` runs hardware detection and the numerical self-test on a node (flows/platform 1.6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemDoctorArgs {
    #[serde(default)]
    pub node_id: String,
}

/// What the doctor job started on: its node and backend. The verdicts arrive as events and in `system.hardware`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemDoctorResult {
    pub node_id: String,
    pub backend: String,
}

/// `system.bundle` packs logs, the hardware profile and versions into one zip at a path the user picks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBundleArgs {
    pub path: String,
}

/// The written bundle: where it is and how big.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBundleResult {
    pub path: String,
    pub size_bytes: u64,
}

pub fn describe_system_hardware_args() -> TypeDescription {
    command_type(
        "SystemHardwareArgs",
        HARDWARE_COMMAND_ID,
        "The node to read; empty for the embedded node.",
        vec![field(
            "node_id",
            FieldKind::String,
            OPTIONAL,
            "The node to read; empty for the embedded node.",
        )],
    )
}

pub fn describe_system_hardware_result() -> TypeDescription {
    let fields = vec![
        field("node_id", FieldKind::String, REQUIRED, "The node the profile is of."),
        field(
            "hardware",
            FieldKind::Proto(HARDWARE_PROFILE_MESSAGE.to_string()),
            REQUIRED,
            "The hardware profile, with the last self-test verdicts.",
        ),
    ];
    command_type(
        "SystemHardwareResult",
        HARDWARE_COMMAND_ID,
        "A node's hardware profile.",
        fields,
    )
}

pub fn describe_system_env_list_args() -> TypeDescription {
    command_type(
        "SystemEnvListArgs",
        ENV_LIST_COMMAND_ID,
        "The node whose environments to list; empty for the embedded node.",
        vec![field(
            "node_id",
            FieldKind::String,
            OPTIONAL,
            "The node whose environments to list; empty for the embedded node.",
        )],
    )
}

pub fn describe_system_env_list_result() -> TypeDescription {
    let fields = vec![field(
        "environments",
        list_of(FieldKind::Proto(ENVIRONMENT_INFO_MESSAGE.to_string())),
        REQUIRED,
        "Every environment on the node, with its lock hash.",
    )];
    command_type(
        "SystemEnvListResult",
        ENV_LIST_COMMAND_ID,
        "The node's uv environments.",
        fields,
    )
}

pub fn describe_system_env_install_args() -> TypeDescription {
    let fields = vec![
        field(
            "node_id",
            FieldKind::String,
            OPTIONAL,
            "The node to sync on; empty for the embedded node.",
        ),
        field(
            "backend",
            enum_of(BACKENDS),
            REQUIRED,
            "The backend whose environment to create or sync.",
        ),
    ];
    command_type(
        "SystemEnvInstallArgs",
        ENV_INSTALL_COMMAND_ID,
        "Create or sync the environment of one backend.",
        fields,
    )
}

pub fn describe_system_env_install_result() -> TypeDescription {
    let fields = vec![field(
        "plan",
        FieldKind::Type(ENVIRONMENT_INSTALL_PLAN_TYPE.to_string()),
        REQUIRED,
        "What the install would create or sync, with the disk check.",
    )];
    command_type(
        "SystemEnvInstallResult",
        ENV_INSTALL_COMMAND_ID,
        "The install's plan; a real run also starts the job.",
        fields,
    )
}

pub fn describe_system_doctor_args() -> TypeDescription {
    command_type(
        "SystemDoctorArgs",
        DOCTOR_COMMAND_ID,
        "The node to check; empty for the embedded node.",
        vec![field(
            "node_id",
            FieldKind::String,
            OPTIONAL,
            "The node to check; empty for the embedded node.",
        )],
    )
}

pub fn describe_system_doctor_result() -> TypeDescription {
    let fields = vec![
        field(
            "node_id",
            FieldKind::String,
            REQUIRED,
            "The node the self-test runs on.",
        ),
        field(
            "backend",
            FieldKind::String,
            REQUIRED,
            "The backend the self-test runs on; the verdicts arrive as events and in system.hardware.",
        ),
    ];
    command_type(
        "SystemDoctorResult",
        DOCTOR_COMMAND_ID,
        "What the doctor job started on.",
        fields,
    )
}

pub fn describe_system_bundle_args() -> TypeDescription {
    let fields = vec![field(
        "path",
        FieldKind::String,
        REQUIRED,
        "Where to write the zip, a path the user picks; a file already there is refused. The bundle never includes plugin-data.",
    )];
    command_type(
        "SystemBundleArgs",
        BUNDLE_COMMAND_ID,
        "Pack logs, the hardware profile and versions into one file.",
        fields,
    )
}

pub fn describe_system_bundle_result() -> TypeDescription {
    let fields = vec![
        field("path", FieldKind::String, REQUIRED, "The written zip."),
        field("size_bytes", FieldKind::Integer, REQUIRED, "The zip's size in bytes."),
    ];
    command_type("SystemBundleResult", BUNDLE_COMMAND_ID, "The written bundle.", fields)
}
