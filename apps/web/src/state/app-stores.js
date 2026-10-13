// The app's shared state (layout frontend-and-plugins.md 8.1, state/app-stores.js). Each store is a plain value with
// get, set and subscribe (@rastai/plugin's createStore). Components read them with useStore.
// Server state (what the daemon said) is in these stores; computed view state is in the components.

import { createStore } from "@rastai/plugin";

import { DEFAULT_MODE } from "../constants.js";
import { emptySettings } from "./settings.js";

/**
 * @typedef {object} AppStores
 * @property {import("@rastai/plugin").Store} settings     the settings.get result: { values, sources }
 * @property {import("@rastai/plugin").Store} commands     CommandDescriptor[], every section (ListCommands)
 * @property {import("@rastai/plugin").Store} plugins      PluginSummary[] (plugin.list)
 * @property {import("@rastai/plugin").Store} connection   { status, problem }: the event stream's state
 * @property {import("@rastai/plugin").Store} route        the parsed route of the address bar
 * @property {import("@rastai/plugin").Store} layouts      { [sectionId]: layout tree } (workspace/layout-tree.js)
 * @property {import("@rastai/plugin").Store} context      { [pluginId]: { [key]: value } } from setContext
 * @property {import("@rastai/plugin").Store} display      { mode, theme }: the window's resolved display
 * @property {import("@rastai/plugin").Store} jobs         JobSummary[] (job.list), kept fresh by job.state_changed
 * @property {import("@rastai/plugin").Store} nodes        NodeSummary[] (node.list)
 * @property {import("@rastai/plugin").Store} telemetry    the last NodeTelemetry of each node, from node.heartbeat
 * @property {import("@rastai/plugin").Store} egress       the egress tracker's entries (frame/egress-state.js)
 * @property {import("@rastai/plugin").Store} notices      ToastEntry[] for the toast list
 * @property {import("@rastai/plugin").Store} palette      { open: boolean, query: string }
 * @property {import("@rastai/plugin").Store} overlay      "" | "shortcuts" | "egress": the open dialog
 * @property {import("@rastai/plugin").Store} signedOut    true once the window has no session
 */

/**
 * The stores, each with its empty value. The boot steps fill the server-backed ones.
 * @param {{ mode: import("@rastai/ui").UiMode }} [start]
 * @returns {AppStores}
 */
export function createAppStores(start = { mode: DEFAULT_MODE }) {
  return {
    settings: createStore(emptySettings()),
    commands: createStore([]),
    plugins: createStore([]),
    connection: createStore({ status: "connecting", problem: null }),
    route: createStore({ sectionId: "home", path: [], query: {} }),
    layouts: createStore({}),
    context: createStore({}),
    display: createStore({ mode: start.mode, theme: null }),
    jobs: createStore([]),
    nodes: createStore([]),
    telemetry: createStore({}),
    egress: createStore([]),
    notices: createStore([]),
    palette: createStore({ open: false, query: "" }),
    overlay: createStore(""),
    signedOut: createStore(false),
  };
}
