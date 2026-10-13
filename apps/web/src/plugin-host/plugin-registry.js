// What the registered UIs and the enabled plugins offer the shell (layout frontend-and-plugins.md 2.6, 10.2): the
// sections, the panels and each manifest's contributions of one kind. Reads only: the loader registers UIs, and
// nothing else registers one.

import { registeredPluginUi } from "@rastai/plugin";

/** @typedef {import("@rastai/plugin").Contribution} Contribution */
/** @typedef {import("@rastai/plugin").Panel} Panel */
/** @typedef {import("@rastai/plugin").Section} Section */

/**
 * The section a registered UI offers for an id, or null when no registered UI has it.
 * @param {string} sectionId
 * @param {string[]} pluginIds   the registered plugin ids the window loaded, core first
 * @returns {Section | null}
 */
export function sectionById(sectionId, pluginIds) {
  for (const pluginId of pluginIds) {
    const section = registeredPluginUi(pluginId)?.sections.find((candidate) => candidate.id === sectionId);
    if (section !== undefined) {
      return section;
    }
  }
  return null;
}

/**
 * The panel with this id, from any registered UI, or null.
 * @param {string} panelId
 * @param {string[]} pluginIds
 * @returns {Panel | null}
 */
export function panelById(panelId, pluginIds) {
  for (const pluginId of pluginIds) {
    const panel = registeredPluginUi(pluginId)?.panels.find((candidate) => candidate.id === panelId);
    if (panel !== undefined) {
      return panel;
    }
  }
  return null;
}

/**
 * Every enabled plugin's entries of one contributes kind (contracts 5.5), as the host hands them to a UI.
 * @param {any[]} plugins    PluginSummary[] from plugin.list
 * @param {string} kind      a contributes key, e.g. "lenses"
 * @returns {Contribution[]}
 */
export function contributionsOf(plugins, kind) {
  return plugins
    .filter((plugin) => plugin.state === "enabled")
    .flatMap((plugin) => {
      /** @type {any[]} */
      const entries = plugin.manifest?.contributes?.[kind] ?? [];
      return entries.map((entry) => ({
        pluginId: plugin.plugin_id,
        id: String(entry.id ?? ""),
        title: String(entry.title ?? entry.id ?? ""),
        data: entry,
      }));
    });
}
