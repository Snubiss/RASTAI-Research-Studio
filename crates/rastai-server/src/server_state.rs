//! `Server`, the plain struct every handler reads, shared as `Arc<Server>` (layout section 8.2). Tables sit behind
//! std locks that are never held across an `.await`.
//!
//! M1-26 brought the fields startup and sign-in need; M1-27 the command registry, the settings layers that sit above
//! the files, the idempotency locks and the port; M1-32 the plugin host; M1-30 the MCP sessions; M1-28 the bus, the
//! daemon's own event log and the tailed runs; SEC-01 the plugin-host lock; M1-47 the daemon's identity, the node
//! links and the scheduler. The gateway and the rest join with the tasks that build them.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock, RwLock};

use rastai_core::commands::registry::{CommandRegistry, load_core_commands};
use rastai_core::paths::DataPaths;
use rastai_core::settings::{ResolvedSettings, SettingsLayer, setting_u64};
use rastai_core::time::now_unix_ms;
use rastai_formats::commands::{describe_arguments, describe_result};
use rastai_formats::describe::json_schema::json_schema_for;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::describe::types::TypeDescription;
use rastai_mcp::server::session::McpSession;
use rastai_node::identity::NodeIdentity;
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_store::store::Store;
use tokio::sync::{oneshot, watch};

use crate::commands::journal::IdempotencyKeyLock;
use crate::events::bus::{BUS_BUFFER_EVENTS, Bus, new_bus};
use crate::events::daemon_events::{DaemonLog, new_daemon_log};
use crate::http::sessions::AuthState;
use crate::nodes::node_links::NodeLinks;
use crate::plugin_host::load::PluginHost;
use crate::scheduler::queue::{Scheduler, new_scheduler};

/// How many events the bus keeps for clients that reconnect (contracts 7.2 and 11.2).
const BUS_BUFFER_SETTING: &str = "daemon.bus_buffer_events";

pub struct Server {
    pub paths: DataPaths,
    /// The settings in effect outside any project: the defaults, the user's `settings.json`, then the overrides.
    pub settings: RwLock<ResolvedSettings>,
    /// The environment and `--set` layers, read once at startup. They sit above the files, so `settings.set`
    /// resolves with them again after it writes one (contracts 11.1).
    pub settings_overrides: Vec<SettingsLayer>,
    pub store: Store,
    /// Every command, core and plugin, with its argument and result schemas (layout 10.2). Rebuilt at every start.
    pub registry: RwLock<CommandRegistry>,
    /// Every plugin discovery found, with its state and diagnostics (plugin_host/). Rebuilt at every start.
    pub plugins: RwLock<PluginHost>,
    /// The plugin-host lock of contracts 8.2 ("Token creators and revocation"): `token.create` for a `plugin:`
    /// creator holds it from its check that the plugin is enabled until its insert commits, and `disable_plugin`
    /// from the state change until its tokens are revoked and marked in the gate's cache, so no plugin-made token
    /// outlives the plugin's disable. A tokio mutex, because both hold it across a write; `plugins` stays a std lock
    /// for the short reads in between.
    pub plugin_host_lock: tokio::sync::Mutex<()>,
    /// A test seam, always `None` in a running daemon: when a test sets it, the next `token.create` stops between
    /// its checks and its insert, still holding `plugin_host_lock` for a `plugin:` creator, until the test lets it
    /// go. tests/token_creators.rs puts a disable in that window, so a create that let the lock go early fails it.
    pub token_create_pause: Mutex<Option<TokenCreatePause>>,
    /// A test seam, always `None` in a running daemon: when a test sets it, the next launch stops just before its
    /// `StartJob` call, still holding whatever lock the launch holds there, until the test lets it go
    /// (scheduler/launch.rs). tests/node_link.rs uses it to show a heartbeat is applied while a launch is in flight.
    pub start_job_pause: Mutex<Option<StartJobPause>>,
    /// Login codes, sessions, the owner token's hash and the scoped tokens' hashes.
    pub auth: AuthState,
    /// The daemon-wide bus: the last `daemon.bus_buffer_events` events and every `Subscribe` stream's source.
    pub bus: Bus,
    /// `<data>/daemon/events.log`, written by `record_daemon_event` only. A tokio mutex: it is held across the index
    /// write, so the daemon's records are indexed in log order.
    pub daemon_log: tokio::sync::Mutex<DaemonLog>,
    /// The runs with a live tail (indexing/run_tail.rs): a run is in it from its tail's start to its end, so a
    /// reconnect never starts a second tail of one run.
    pub tailed_run_ids: Mutex<HashSet<String>>,
    /// One lock per (principal, idempotency key) while a command with that key runs (commands/journal.rs).
    pub idempotency_locks: Mutex<Vec<IdempotencyKeyLock>>,
    /// The open MCP sessions of `/mcp` (http/mcp_endpoint.rs), in memory only: a restart ends them all.
    pub mcp_sessions: Mutex<Vec<McpSession>>,
    /// The loopback port, set once the listener is bound; `token.login_code.create` reports it.
    pub http_port: OnceLock<u16>,
    /// `daemon.key`, through rastai_node::identity (the one Ed25519 wrapper): it signs the node handshakes, and its
    /// public key is the daemon id (contracts 8.6).
    pub identity: NodeIdentity,
    /// Every node the daemon knows and how its connection stands (nodes/node_links.rs).
    pub nodes: NodeLinks,
    /// The job queue's wake-up and the job lock (scheduler/queue.rs).
    pub scheduler: Scheduler,
    /// Set to true to stop the daemon (`request_shutdown`): the signal handler and `system.shutdown` do. The
    /// listeners watch it.
    pub shutdown: watch::Sender<bool>,
}

/// How a paused `token.create` tells the test it has stopped (`reached`), and waits for the test to let it go on
/// (`resume`, or the test dropping its sender).
pub struct TokenCreatePause {
    pub reached: oneshot::Sender<()>,
    pub resume: oneshot::Receiver<()>,
}

/// How a launch held by the `start_job_pause` seam tells the test it has stopped (`reached`), and waits for the test to
/// let it go on (`resume`, or the test dropping its sender).
pub struct StartJobPause {
    pub reached: oneshot::Sender<()>,
    pub resume: oneshot::Receiver<()>,
}

/// What `build_server` puts together; each part is made by its own startup step (daemon.rs).
pub struct ServerParts {
    pub paths: DataPaths,
    pub settings: ResolvedSettings,
    pub settings_overrides: Vec<SettingsLayer>,
    pub store: Store,
    pub auth: AuthState,
    pub identity: NodeIdentity,
    pub nodes: NodeLinks,
}

pub fn build_server(parts: ServerParts) -> Server {
    let ServerParts {
        paths,
        settings,
        settings_overrides,
        store,
        auth,
        identity,
        nodes,
    } = parts;
    let (shutdown, _first_receiver) = watch::channel(false);
    let bus_buffer_events = usize::try_from(setting_u64(&settings, BUS_BUFFER_SETTING)).unwrap_or(BUS_BUFFER_EVENTS);
    Server {
        paths,
        settings: RwLock::new(settings),
        settings_overrides,
        store,
        registry: RwLock::new(core_command_registry()),
        plugins: RwLock::new(PluginHost::default()),
        plugin_host_lock: tokio::sync::Mutex::new(()),
        token_create_pause: Mutex::new(None),
        start_job_pause: Mutex::new(None),
        auth,
        bus: new_bus(bus_buffer_events, now_unix_ms()),
        daemon_log: tokio::sync::Mutex::new(new_daemon_log()),
        tailed_run_ids: Mutex::new(HashSet::new()),
        idempotency_locks: Mutex::new(Vec::new()),
        mcp_sessions: Mutex::new(Vec::new()),
        http_port: OnceLock::new(),
        identity,
        nodes,
        scheduler: new_scheduler(),
        shutdown,
    }
}

/// The core table, with each command's argument and result types and their JSON Schemas filled in from
/// rastai-formats' descriptions (layout 10.2): what `ListCommands`, MCP's `inputSchema` and the palette's forms read.
fn core_command_registry() -> CommandRegistry {
    let all_types = describe_all_types();
    let mut registry = load_core_commands();
    for command in &mut registry.commands {
        fill_command_types(command, &all_types);
    }
    registry
}

fn fill_command_types(command: &mut CommandDescriptor, all_types: &[TypeDescription]) {
    if let Some(arguments) = describe_arguments(&command.command_id) {
        command.arguments_schema_json = json_schema_for(&arguments, all_types).to_string();
        command.arguments_type = arguments.name;
    }
    if let Some(result) = describe_result(&command.command_id) {
        command.result_schema_json = json_schema_for(&result, all_types).to_string();
        command.result_type = result.name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_command_carries_its_types_and_schemas() {
        let registry = core_command_registry();
        assert!(!registry.commands.is_empty());
        for command in &registry.commands {
            assert!(command.arguments_type.ends_with("Args"), "{}", command.command_id);
            assert!(command.result_type.ends_with("Result"), "{}", command.command_id);
            let arguments: serde_json::Value = serde_json::from_str(&command.arguments_schema_json).unwrap_or_default();
            assert_eq!(arguments["type"], "object", "{}", command.command_id);
            let result: serde_json::Value = serde_json::from_str(&command.result_schema_json).unwrap_or_default();
            assert_eq!(result["title"], command.result_type.as_str(), "{}", command.command_id);
        }
    }
}
