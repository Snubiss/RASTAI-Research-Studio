// What an in-process UI registers (layout frontend-and-plugins.md 7.2 and 10.2): a first-party plugin's module
// exports one PluginUi (`psychologyUi`), and the app's loader passes it to registerPluginUi; the core sections
// register the same way under the pseudo plugin id `core`. The app then matches the registered ids against the
// manifest (RRS-P0101, RRS-P0102) and builds its sidebar and slots from them. A section, panel, lens view, metric
// view or settings page is any object with an id and a render function.

import { CORE_PLUGIN_ID } from "./in-process-host.js";

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @typedef {object} Panel
 * @property {string} id                 `<plugin_id>.<name>`, e.g. "psychology.live"
 * @property {(props: { params: Object<string, string> }) => ReactNode} render
 */

/**
 * @typedef {object} Section
 * @property {string} id                 the plugin's own id (a core section's is its own name, e.g. "home")
 * @property {(props: { path: string[], query: Object<string, string> }) => ReactNode} render
 */

/**
 * @typedef {object} LensView
 * @property {string} id
 * @property {(props: { sessionId: string, tokenIndex: number | null, readoutJson: string }) => ReactNode} render
 */

/**
 * @typedef {object} MetricView
 * @property {string} id
 * @property {(props: { resultJson: string, compareToJson: string | null }) => ReactNode} render
 */

/**
 * @typedef {object} SettingsPage
 * @property {string} id
 * @property {() => ReactNode} render
 */

/**
 * @typedef {object} PluginUi
 * @property {string} pluginId
 * @property {Section[]} sections
 * @property {Panel[]} panels
 * @property {LensView[]} lensViews
 * @property {MetricView[]} metricViews
 * @property {SettingsPage[]} settingsPages
 */

// The lists whose ids are namespaced `<plugin_id>.<name>` (layout decision 12).
const NAMESPACED_LISTS = ["panels", "lensViews", "metricViews", "settingsPages"];

/** @type {Map<string, PluginUi>} */
const registeredUis = new Map();

/**
 * Registers a plugin's in-process UI, once per plugin id. A second registration is refused, whatever its source,
 * so nothing can replace an earlier one: not a plugin, and not the core sections under the id `core`. Throws on a
 * malformed UI or a repeated id; the app's loader turns that into RRS-P4105 and starts without the plugin (layout
 * 10.2). The app calls it from one place only, its plugin loader.
 * @param {PluginUi} ui
 */
export function registerPluginUi(ui) {
  const problem = pluginUiProblem(ui);
  if (problem !== null) {
    throw new Error(`The plugin UI of ${String(ui?.pluginId)} can't be registered: ${problem}`);
  }
  if (registeredUis.has(ui.pluginId)) {
    throw new Error(`The plugin UI of ${ui.pluginId} is already registered; a registration is never replaced.`);
  }
  registeredUis.set(ui.pluginId, ui);
}

/**
 * @param {string} pluginId
 * @returns {PluginUi | null} what the plugin registered, or null
 */
export function registeredPluginUi(pluginId) {
  return registeredUis.get(pluginId) ?? null;
}

/**
 * What is wrong with a PluginUi, or null when nothing is.
 * @param {PluginUi} ui
 * @returns {string | null}
 */
export function pluginUiProblem(ui) {
  if (typeof ui?.pluginId !== "string" || ui.pluginId === "") {
    return "pluginId must be a non-empty string";
  }
  const lists = {
    sections: ui.sections,
    panels: ui.panels,
    lensViews: ui.lensViews,
    metricViews: ui.metricViews,
    settingsPages: ui.settingsPages,
  };
  for (const [listName, list] of Object.entries(lists)) {
    const problem = listProblem(ui.pluginId, listName, list);
    if (problem !== null) {
      return problem;
    }
  }
  const strangeSection = ui.sections.find((section) => !isOwnSectionId(ui.pluginId, section.id));
  if (strangeSection) {
    return `the section ${strangeSection.id} isn't the plugin's own id ${ui.pluginId}`;
  }
  return null;
}

/**
 * Checks one list: every entry has an id and a render function, no id is there twice, and namespaced ids start
 * with the plugin's id.
 * @param {string} pluginId
 * @param {string} listName
 * @param {any} list
 * @returns {string | null}
 */
function listProblem(pluginId, listName, list) {
  if (!Array.isArray(list)) {
    return `${listName} must be a list`;
  }
  /** @type {Set<string>} */
  const seen = new Set();
  for (const entry of list) {
    if (typeof entry?.id !== "string" || typeof entry?.render !== "function") {
      return `every entry of ${listName} needs an id and a render function`;
    }
    if (seen.has(entry.id)) {
      return `${listName} has the id ${entry.id} twice`;
    }
    seen.add(entry.id);
    if (NAMESPACED_LISTS.includes(listName) && !entry.id.startsWith(`${pluginId}.`)) {
      return `the id ${entry.id} in ${listName} must start with ${pluginId}.`;
    }
  }
  return null;
}

/**
 * @param {string} pluginId
 * @param {string} sectionId
 * @returns {boolean}
 */
function isOwnSectionId(pluginId, sectionId) {
  return pluginId === CORE_PLUGIN_ID || sectionId === pluginId;
}
