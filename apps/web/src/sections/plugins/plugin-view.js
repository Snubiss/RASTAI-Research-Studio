// What the Plugins list says about one plugin (ui-design 6.8). The daemon's plugin.list says what is installed, its state
// and the permissions it asks for and was granted; these functions only turn that into words and a tone.

import { pluginDiagnostic } from "@rastai/plugin";

/** @type {Object<string, string>} */
const SOURCE_WORDS = {
  preinstalled: "Comes with Rastai",
  installed: "Installed by you",
  dev: "A development folder",
};
// The PluginState names (contracts 9.7). A state not in these lists is neutral, so a new state shows no false alarm.
const ENABLED_STATE = "enabled";
const WARNING_STATES = ["awaiting_trust"];
const ERROR_STATES = ["incompatible", "failed"];

/**
 * The name people know a plugin by.
 * @param {any} plugin   a PluginSummary
 * @returns {string}
 */
export function pluginTitle(plugin) {
  return plugin.manifest?.display_name || plugin.plugin_id;
}

/**
 * Where the daemon found the plugin, in words.
 * @param {string} source
 * @returns {string}
 */
export function sourceWord(source) {
  return SOURCE_WORDS[source] ?? "Unknown source";
}

/**
 * The trust line: whether every permission the plugin asks for was granted, or how many are still waiting.
 * @param {any} plugin
 * @returns {string}
 */
export function trustLine(plugin) {
  const requested = plugin.manifest?.permissions ?? [];
  const granted = plugin.granted_permissions ?? [];
  if (requested.length === 0) {
    return "Asks for no permissions";
  }
  const waiting = requested.filter((/** @type {string} */ permission) => !granted.includes(permission));
  return waiting.length === 0
    ? "Every permission it asks for is granted"
    : `${waiting.length} of ${requested.length} permissions wait for your review`;
}

/**
 * The state pill's tone: enabled is ok, a plugin that waits for the owner's trust is a warning, and one that is
 * incompatible or failed is an error.
 * @param {string} state
 * @returns {"ok" | "warning" | "error" | "neutral"}
 */
export function stateTone(state) {
  if (state === ENABLED_STATE) {
    return "ok";
  }
  if (WARNING_STATES.includes(state)) {
    return "warning";
  }
  if (ERROR_STATES.includes(state)) {
    return "error";
  }
  return "neutral";
}

/**
 * The diagnostic that says why a plugin is incompatible, from the daemon's own list when it has one.
 * @param {any} plugin
 * @returns {any}   a Diagnostic, or null when the plugin has no problem to show
 */
export function firstProblem(plugin) {
  const diagnostics = plugin.diagnostics ?? [];
  if (diagnostics.length > 0) {
    return diagnostics[0];
  }
  return stateTone(plugin.state) === "error" ? pluginDiagnostic("RRS-P1002", {}) : null;
}
