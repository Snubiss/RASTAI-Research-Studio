//! `CommandRow`, one row of the core command table (layout rust-control-plane.md 5.2). The table itself, `COMMANDS`,
//! arrives with M1-21; `wire.rs` turns a row into the proto `CommandDescriptor` the registry holds.

use rastai_proto::commands::v1::CommandKind;
use rastai_proto::common::v1::Stability;

use crate::scopes::Scope;

/// One row of the core table: the one allowed in-process twin of the proto `CommandDescriptor`, because a const
/// table can't hold Strings. Same field names as the message; it leaves out only what registration fills in
/// (`plugin_id`, `arguments_type`, `result_type` and the two schema strings). `tests/wire_round_trip.rs` pins the
/// field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRow {
    pub command_id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub section: &'static str,
    pub scope: Scope,
    /// The generated enum, not one of our own.
    pub kind: CommandKind,
    /// The generated enum. Every row is `Preview` until Phase 2 (owner decision N3).
    pub stability: Stability,
    pub undoable: bool,
    /// Shown in Basic mode.
    pub basic: bool,
    /// Empty when the command has none.
    pub default_shortcut: &'static str,
    pub experimental: bool,
    pub since_version: &'static str,
}
