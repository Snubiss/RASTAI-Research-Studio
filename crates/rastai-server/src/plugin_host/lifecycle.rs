//! Plugin state changes and `plugin.state_changed` (flows/platform.md 7.2; contracts 7.3, 9.7).
//!
//! Every change goes through `change_plugin_state`, which allows only the moves of the state machine
//! (`plugin_transition_allowed`, rastai-core) and gives back the `plugin.state_changed` record of the move, with the
//! reason as its diagnostic. "Disabled with a diagnostic, never a crash": `incompatible`, `disabled` and `failed` are
//! ordinary states the Plugins section shows with their fix.
//!
//! What is persisted: an installed plugin's state in its `plugins` row (`save_plugin_row`), when the user decided it
//! (a load that enabled it, or `disable_plugin`); the host's own outcomes are decided again at every start (load.rs).
//! A dev plugin has no row and is found again at every start. The `plugin.state_changed` record itself goes into the
//! daemon log and onto the bus: `apply_state` (load.rs) keeps it in `PluginHost.state_changes` while it holds the
//! host's lock, and `record_plugin_state_changes` sends what is kept through `record_daemon_event` once that lock is
//! released, since an append is awaited. Every path that changes a state calls it, and the first runs after
//! `replay_daemon_log` (daemon.rs), so a new record never lands in the log ahead of the replay.

use rastai_core::states::{plugin_state_name, plugin_transition_allowed};
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_diag::logger::{log_info, log_warn};
use rastai_proto::common::v1::PluginState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{EventPayload, EventRecord, PluginStateChanged};
use rastai_store::index::kept_tables::PluginRow;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;

use crate::events::daemon_events::record_daemon_event;
use crate::plugin_host::load::{LoadedPlugin, PluginSource, plugin_source_name, write_plugin_host};
use crate::server_state::Server;

/// How many records `PluginHost.state_changes` keeps until `record_plugin_state_changes` takes them; the oldest go
/// first. Each change is taken right after it is made, so the cap only bounds a caller that forgot to.
const MAX_PENDING_STATE_CHANGES: usize = 1000;
const PLUGIN_STATE_CHANGED_EVENT: &str = "plugin.state_changed";

/// Moves the plugin to `to_state` with its reasons, and returns the record of the move. Staying in the same state
/// only refreshes the reasons and records nothing. A move the state machine doesn't allow is a bug in the caller: it
/// is logged and the plugin keeps its state.
pub(crate) fn change_plugin_state(
    plugin: &mut LoadedPlugin,
    to_state: PluginState,
    diagnostics: Vec<Diagnostic>,
) -> Option<PluginStateChanged> {
    let from_state = plugin.state;
    if from_state == to_state {
        plugin.diagnostics = diagnostics;
        return None;
    }
    if !plugin_transition_allowed(from_state, to_state) {
        log_warn(
            "a plugin state change the state machine doesn't allow was ignored",
            &[
                ("plugin_id", &plugin.plugin_id),
                ("from_state", plugin_state_name(from_state)),
                ("to_state", plugin_state_name(to_state)),
            ],
        );
        return None;
    }
    plugin.state = to_state;
    plugin.diagnostics = diagnostics;
    Some(PluginStateChanged {
        plugin_id: plugin.plugin_id.clone(),
        from_state: from_state as i32,
        to_state: to_state as i32,
        diagnostic: reason_of(&plugin.diagnostics).map(diagnostic_to_proto),
    })
}

/// Keeps the record for the daemon log and logs it.
pub(crate) fn record_state_change(state_changes: &mut Vec<PluginStateChanged>, change: PluginStateChanged) {
    let to_state = PluginState::try_from(change.to_state).unwrap_or(PluginState::Unspecified);
    let from_state = PluginState::try_from(change.from_state).unwrap_or(PluginState::Unspecified);
    let code = change
        .diagnostic
        .as_ref()
        .map(|item| item.code.clone())
        .unwrap_or_default();
    log_info(
        "plugin.state_changed",
        &[
            ("plugin_id", &change.plugin_id),
            ("from_state", plugin_state_name(from_state)),
            ("to_state", plugin_state_name(to_state)),
            ("code", &code),
        ],
    );
    if state_changes.len() >= MAX_PENDING_STATE_CHANGES {
        state_changes.remove(0);
    }
    state_changes.push(change);
}

/// Sends every kept `plugin.state_changed` record, oldest first, to the daemon log, which indexes and publishes it.
/// A record that can't be logged is a warning: the plugin's state in memory is right either way, and the next start
/// decides it again.
pub(crate) async fn record_plugin_state_changes(server: &Server) {
    let changes = take_state_changes(server);
    for change in changes {
        let plugin_id = change.plugin_id.clone();
        if let Err(error) = record_daemon_event(server, state_changed_record(change)).await {
            log_warn(
                "a plugin.state_changed record could not be logged and indexed",
                &[("plugin_id", &plugin_id), ("code", error.code())],
            );
        }
    }
}

/// Taking and appending are two steps. Today one task at a time changes plugin states (the start, then tests), so
/// the log keeps the order of the moves. When `plugin.enable` and `plugin.disable` (M8) can run at once, two callers
/// could append their takes out of order: hold the daemon log's lock across the take then, or order by sequence.
fn take_state_changes(server: &Server) -> Vec<PluginStateChanged> {
    let mut host = write_plugin_host(server);
    std::mem::take(&mut host.state_changes)
}

/// `emitted_unix_ms` is left for `record_daemon_event` to fill at the append, which follows the move within the same
/// call, so the gap is the lock release and nothing a reader would see.
fn state_changed_record(change: PluginStateChanged) -> EventRecord {
    EventRecord {
        event_type: PLUGIN_STATE_CHANGED_EVENT.to_string(),
        payload: Some(EventPayload {
            payload: Some(Payload::PluginStateChanged(change)),
        }),
        ..EventRecord::default()
    }
}

/// The `plugins` row of an installed or preinstalled plugin; None for a dev plugin, which has none.
pub(crate) fn plugin_row(plugin: &LoadedPlugin) -> Option<PluginRow> {
    if plugin.source == PluginSource::Dev {
        return None;
    }
    Some(PluginRow {
        plugin_id: plugin.plugin_id.clone(),
        version: plugin.version.clone(),
        state: plugin_state_name(plugin.state).to_string(),
        content_hash: plugin.content_hash.clone(),
        granted_permissions: plugin.granted_permissions.clone(),
        source: plugin_source_name(plugin.source).to_string(),
    })
}

/// Writes the row, when there is one. A write that fails is logged: the plugin's state in memory is still right, and
/// the next start checks the plugin again.
pub(crate) async fn save_plugin_row(server: &Server, row: Option<PluginRow>) {
    let Some(row) = row else {
        return;
    };
    let plugin_id = row.plugin_id.clone();
    if let Err(error) = write_index(&server.store.index, IndexWrite::UpsertPlugin(row)).await {
        log_warn(
            "a plugin's state could not be saved",
            &[("plugin_id", &plugin_id), ("code", error.code())],
        );
    }
}

/// The diagnostic the record carries: the first error, else the first diagnostic of any severity.
fn reason_of(diagnostics: &[Diagnostic]) -> Option<&Diagnostic> {
    diagnostics
        .iter()
        .find(|item| item.severity == Severity::Error)
        .or_else(|| diagnostics.first())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::diagnostic;
    use std::path::PathBuf;

    fn dev_plugin(state: PluginState) -> LoadedPlugin {
        LoadedPlugin {
            plugin_id: "acme_tools".to_string(),
            version: "1.0.0".to_string(),
            source: PluginSource::Dev,
            folder: PathBuf::new(),
            manifest: None,
            state,
            granted_permissions: Vec::new(),
            content_hash: String::new(),
            diagnostics: Vec::new(),
            manifest_problems: Vec::new(),
            event_handlers: Vec::new(),
        }
    }

    /// One row per move a load or the user makes: (from, to, recorded and taken).
    const MOVES: &[(PluginState, PluginState, bool)] = &[
        (PluginState::Installed, PluginState::Enabled, true),
        (PluginState::Installed, PluginState::Incompatible, true),
        (PluginState::Installed, PluginState::Disabled, true),
        (PluginState::Installed, PluginState::AwaitingTrust, true),
        (PluginState::Installed, PluginState::Failed, true),
        (PluginState::Enabled, PluginState::Disabled, true),
        (PluginState::Enabled, PluginState::Incompatible, true),
        (PluginState::Disabled, PluginState::Enabled, true),
        (PluginState::Enabled, PluginState::Enabled, false),
        // Not moves of the state machine: refused, the plugin keeps its state.
        (PluginState::Failed, PluginState::Enabled, false),
        (PluginState::Incompatible, PluginState::Enabled, false),
        (PluginState::Uninstalled, PluginState::Enabled, false),
    ];

    #[test]
    fn each_move_is_taken_or_refused_as_the_state_machine_says() {
        for (from_state, to_state, recorded) in MOVES {
            let mut plugin = dev_plugin(*from_state);
            let reason = vec![diagnostic(
                "RRS-P1002",
                &[("plugin_id", "acme_tools"), ("capability", "x@1")],
            )];
            let change = change_plugin_state(&mut plugin, *to_state, reason);
            assert_eq!(change.is_some(), *recorded, "{from_state:?} -> {to_state:?}");
            let expected_state = if *recorded || from_state == to_state {
                *to_state
            } else {
                *from_state
            };
            assert_eq!(plugin.state, expected_state, "{from_state:?} -> {to_state:?}");
            if let Some(change) = change {
                assert_eq!(change.from_state, *from_state as i32);
                assert_eq!(change.to_state, *to_state as i32);
                assert_eq!(change.diagnostic.unwrap().code, "RRS-P1002");
            }
        }
    }

    #[test]
    fn the_record_carries_the_first_error() {
        let mut plugin = dev_plugin(PluginState::Installed);
        let reasons = vec![
            diagnostic("RRS-P2005", &[("plugin_id", "acme_tools")]),
            diagnostic(
                "RRS-P1001",
                &[("plugin_id", "acme_tools"), ("range", ">=99"), ("api_version", "0.1.0")],
            ),
        ];
        let change = change_plugin_state(&mut plugin, PluginState::Incompatible, reasons).unwrap();
        assert_eq!(change.diagnostic.unwrap().code, "RRS-P1001");
        let mut enabled = dev_plugin(PluginState::Installed);
        let change = change_plugin_state(&mut enabled, PluginState::Enabled, Vec::new()).unwrap();
        assert!(change.diagnostic.is_none());
    }

    #[test]
    fn pending_records_are_bounded() {
        let mut state_changes = Vec::new();
        for index in 0..=MAX_PENDING_STATE_CHANGES {
            let change = PluginStateChanged {
                plugin_id: format!("acme{index}"),
                ..PluginStateChanged::default()
            };
            record_state_change(&mut state_changes, change);
        }
        assert_eq!(state_changes.len(), MAX_PENDING_STATE_CHANGES);
        assert_eq!(state_changes[0].plugin_id, "acme1");
    }

    #[test]
    fn only_installed_plugins_have_a_row() {
        let mut plugin = dev_plugin(PluginState::Enabled);
        assert!(plugin_row(&plugin).is_none());
        plugin.source = PluginSource::Installed;
        let row = plugin_row(&plugin).unwrap();
        assert_eq!((row.state.as_str(), row.source.as_str()), ("enabled", "installed"));
    }
}
