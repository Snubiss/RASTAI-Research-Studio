// The plugin loader (layout frontend-and-plugins.md 10.2): the one place that calls registerPluginUi. It registers the
// core sections under the id `core`, then each enabled first-party plugin whose UI this build has. A UI that fails to
// register is reported and skipped, and the app starts without it (RRS-P4105 in the catalog).

import { CORE_PLUGIN_ID, registerPluginUi } from "@rastai/plugin";

import { coreUi } from "./core-sections.jsx";
import { FIRST_PARTY_UI_LOADERS } from "./first-party.js";
import { pluginDiagnostic } from "@rastai/plugin";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

const ENABLED_STATE = "enabled";
// The catalog code for a plugin UI that won't register (RRS-P4105, layout 10.2).
const CODE_UI_NOT_REGISTERED = "RRS-P4105";

/**
 * Registers the core UI, then the first-party UIs of the enabled plugins. Returns the ids that loaded and a Diagnostic
 * for each one that didn't.
 * @param {any[]} plugins   PluginSummary[] from plugin.list
 * @returns {Promise<{ loadedIds: string[], diagnostics: Diagnostic[] }>}
 */
export async function registerShellUis(plugins) {
  /** @type {string[]} */
  const loaded = [];
  /** @type {Diagnostic[]} */
  const diagnostics = [];
  if (registerOnce(coreUi(), diagnostics)) {
    loaded.push(CORE_PLUGIN_ID);
  }
  const enabledIds = plugins.filter((plugin) => plugin.state === ENABLED_STATE).map((plugin) => plugin.plugin_id);
  for (const pluginId of enabledIds) {
    const loader = FIRST_PARTY_UI_LOADERS[pluginId];
    if (loader === undefined) {
      continue;
    }
    const ui = await loader();
    if (registerOnce(ui, diagnostics)) {
      loaded.push(pluginId);
    }
  }
  return { loadedIds: loaded, diagnostics };
}

/**
 * Registers one UI. A malformed UI, or an id already registered, becomes a Diagnostic and is not registered.
 * @param {any} ui
 * @param {Diagnostic[]} diagnostics   where a failure is added
 * @returns {boolean}   true when it registered
 */
function registerOnce(ui, diagnostics) {
  try {
    registerPluginUi(ui);
    return true;
  } catch (error) {
    const pluginId = String(ui?.pluginId ?? "");
    const reason = error instanceof Error ? error.message : String(error);
    diagnostics.push(pluginDiagnostic(CODE_UI_NOT_REGISTERED, { plugin_id: pluginId, reason }));
    return false;
  }
}
