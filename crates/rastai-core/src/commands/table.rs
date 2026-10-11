//! The core command table (contracts 6.2, 6.4; layout rust-control-plane.md 10.1): one `CommandRow` per core
//! command, and the commands that were retired (layout 11.2). Plugin commands come from manifests at runtime, never
//! from this file.
//!
//! Every row is a contract: it lands in a contract PR with its argument and result types and catalog codes, before
//! the handler (plan 3.2). The first rows, the skeleton's commands, arrived with M1-22; their types are in
//! rastai-formats' `commands/`. A row whose handler isn't built yet still runs: it fails with its area's "not
//! available yet" code (catalog/README.md: `RRS-C0010` for `system`, `command`, `job`, `settings` and `token`,
//! `RRS-T0006` for `train`, `RRS-P1004` for `plugin`, `RRS-N0005` for `node`), so the whole surface exists early
//! (ADR-0017). `tests/command_table.rs` checks every row: id
//! form and core area, no duplicates, scope, kind, section, `since_version`, `preview`, and no argument that takes a
//! global CLI flag.

use rastai_proto::commands::v1::CommandKind;
use rastai_proto::common::v1::Stability;

use crate::commands::descriptor::CommandRow;
use crate::scopes::Scope;

/// The app version the skeleton's commands arrived in.
const SKELETON_VERSION: &str = "0.1.0";

/// One row per core command (contracts 6.4).
pub const COMMANDS: &[CommandRow] = &[
    CommandRow {
        command_id: "system.status",
        title: "Show status",
        summary: "Daemon, nodes, versions and protocol versions.",
        section: "settings",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "system.shutdown",
        title: "Stop the daemon",
        summary: "Stop the daemon gracefully; running jobs keep running and are reattached on the next start.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "command.list",
        title: "List commands",
        summary: "Every registered command with its argument and result schemas.",
        section: "settings",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "job.list",
        title: "List jobs",
        summary: "Jobs and their states, by run or by state.",
        section: "home",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "job.get",
        title: "Show job",
        summary: "One job's run, node, kind and state.",
        section: "home",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    // `read` lets the call reach the handler; contracts 6.4 gives job.cancel "the scope of the job", so the handler
    // also checks that the caller holds the scope of the command that started the job.
    CommandRow {
        command_id: "job.cancel",
        title: "Cancel job",
        summary: "Stop a job at its next step boundary, without a new checkpoint.",
        section: "home",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "train.start",
        title: "Start training",
        summary: "Start a run: a recipe, a design and a dataset.",
        section: "training",
        scope: Scope::Training,
        kind: CommandKind::Job,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "settings.get",
        title: "Show settings",
        summary: "The effective settings, with the layer each key came from.",
        section: "settings",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "settings.set",
        title: "Change a setting",
        summary: "Set one key in the user's or the open project's settings.json.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "token.login_code.create",
        title: "Create a login code",
        summary: "A single-use code that signs a browser in to the UI; it expires after 60 seconds.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "token.create",
        title: "Create an API token",
        summary: "A scoped API token for a script or an agent, shown once.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "token.list",
        title: "List API tokens",
        summary: "API tokens with their names and scopes, never their text.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "token.revoke",
        title: "Revoke an API token",
        summary: "Stop an API token working, at once.",
        section: "settings",
        scope: Scope::SystemAdmin,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "plugin.list",
        title: "List plugins",
        summary: "Installed plugins with their state, manifest and granted permissions.",
        section: "plugins",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "node.list",
        title: "List nodes",
        summary: "Nodes with their state and last heartbeat.",
        section: "remote",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: true,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
    CommandRow {
        command_id: "node.status",
        title: "Show node",
        summary: "One node: its state, last telemetry and unfinished jobs.",
        section: "remote",
        scope: Scope::Read,
        kind: CommandKind::Sync,
        stability: Stability::Preview,
        undoable: false,
        basic: false,
        default_shortcut: "",
        experimental: false,
        since_version: SKELETON_VERSION,
    },
];

/// A command that was replaced. Its id is never reused, `ListCommands` doesn't list it, and `Execute` on it answers
/// `RRS-C0007` with a fix button that runs the replacement (layout 11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredCommand {
    pub id: &'static str,
    pub replaced_by: &'static str,
    /// The app version that retired it.
    pub retired_in: &'static str,
}

/// Commands that were replaced; none yet.
pub const RETIRED_COMMANDS: &[RetiredCommand] = &[];
