//! `plugin.*` (contracts 6.4). `plugin.list` is built: every plugin the host knows, enabled or not, with its state,
//! source, granted permissions, manifest and diagnostics, which the UI reads at every start to register each enabled
//! plugin's sections, panels and settings pages. The rest of the area (install, enable, disable, dev tools) is M8.

use rastai_core::commands::output::CommandOutput;
use rastai_core::states::plugin_state_name;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::commands::plugin::{PluginListArgs, PluginListResult, PluginSummary};
use rastai_formats::commands::result_json;
use serde_json::{Map, Value};

use crate::commands::context::{CommandContext, sync_result};
// The diagnostics' protobuf JSON is the codec's: one writer for the wire and for this result (contracts 4.1).
use crate::http::connect_json::diagnostic_to_json;
use crate::plugin_host::load::{LoadedPlugin, plugin_source_name, read_plugin_host};

/// Every plugin, in the order discovery found them.
pub(crate) async fn list(
    context: &CommandContext<'_>,
    _arguments: PluginListArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let plugins = {
        let host = read_plugin_host(context.server);
        host.plugins.iter().map(plugin_summary).collect()
    };
    let result = PluginListResult { plugins };
    Ok(sync_result(result_json(&result)))
}

/// One row of `plugin.list`. A plugin whose manifest couldn't be read lists an empty object as its manifest.
pub(crate) fn plugin_summary(plugin: &LoadedPlugin) -> PluginSummary {
    let manifest = plugin
        .manifest
        .as_ref()
        .and_then(|manifest| serde_json::to_value(manifest).ok())
        .unwrap_or_else(|| Value::Object(Map::new()));
    PluginSummary {
        plugin_id: plugin.plugin_id.clone(),
        version: plugin.version.clone(),
        state: plugin_state_name(plugin.state).to_string(),
        source: plugin_source_name(plugin.source).to_string(),
        granted_permissions: plugin.granted_permissions.clone(),
        manifest,
        diagnostics: plugin.diagnostics.iter().map(diagnostic_json_value).collect(),
    }
}

/// The codec writes valid JSON for every diagnostic, so the parse back never fails; were it to, the entry would be
/// `null` rather than a guess.
fn diagnostic_json_value(diagnostic: &Diagnostic) -> Value {
    serde_json::from_str(&diagnostic_to_json(&diagnostic_to_proto(diagnostic))).unwrap_or(Value::Null)
}
