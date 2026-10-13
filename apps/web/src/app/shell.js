// The window's shell: the stores, the one event hub, the core host and the functions that keep the stores in step with
// the daemon (layout frontend-and-plugins.md 8.1, 8.3). main.jsx builds it once after the boot steps, and the App reads
// it. Nothing here decides what anything means: it asks the daemon with a command and keeps what came back.

import { showToast } from "@rastai/ui";
import { jobList, listCommands, nodeList, openEventStream, pluginList, settingsGet, settingsSet } from "@rastai/client";

import { coreHostOf } from "../plugin-host/in-process-host.js";
import { resyncShell } from "../events/resync.js";
import { createEventHub } from "../events/event-hub.js";
import { egressReportOf, heldEgress, recordEgress } from "../frame/egress-state.js";
import { watchRoute } from "../routing/navigate.js";
import { createAppStores } from "../state/app-stores.js";
import { globalMode } from "../state/mode.js";
import { emptySettings } from "../state/settings.js";
import { applyTheme, themeTokensFor, watchColorScheme } from "../state/theme.js";
import { pluginDiagnostic } from "@rastai/plugin";

/** @typedef {import("@rastai/client").DaemonClient} DaemonClient */
/** @typedef {import("@rastai/plugin").PluginHost} PluginHost */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("../state/app-stores.js").AppStores} AppStores */
/** @typedef {import("../events/event-hub.js").EventHub} EventHub */

const DARK_SCHEME_QUERY = "(prefers-color-scheme: dark)";
const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

/**
 * @typedef {object} Shell
 * @property {DaemonClient} client
 * @property {AppStores} stores
 * @property {EventHub} hub
 * @property {PluginHost} coreHost            the host of the core sections
 * @property {string[]} loadedPluginIds     the plugin ids whose UIs registered, core first (set once at boot)
 * @property {() => void} start              starts the stream, the route, the OS scheme and the visibility watch
 * @property {() => void} stop
 * @property {(result: any) => void} applySettings   stores settings.get's result and resolves the display
 * @property {() => Promise<void>} refreshSettings
 * @property {() => Promise<void>} refreshCommands
 * @property {() => Promise<void>} refreshJobs
 * @property {() => Promise<void>} refreshNodes
 * @property {() => Promise<void>} refreshPlugins
 * @property {(diagnostic: Diagnostic) => void} notify
 * @property {(mode: "basic" | "advanced") => Promise<void>} setMode   the global Basic/Advanced switch (ui.mode)
 */

/**
 * @param {DaemonClient} client
 * @param {{ openStream?: typeof openEventStream }} [options]   the stream to use; tests pass a fake
 * @returns {Shell}
 */
export function createShell(client, options = {}) {
  const stores = createAppStores();
  const openStream = options.openStream ?? openEventStream;
  const hub = createEventHub({
    openStream: (eventTypes, afterBusSequence, handlers) => openStream(client, eventTypes, afterBusSequence, handlers),
    onStatus: (status, problem) => stores.connection.set({ status, problem }),
    onResync: () => {
      resyncShell(shell);
    },
  });
  /** @type {Shell} */
  const shell = {
    client,
    stores,
    hub,
    coreHost: /** @type {any} */ (null),
    loadedPluginIds: ["core"],
    start: () => startShell(shell),
    stop: () => hub.stop(),
    applySettings: (result) => applySettings(shell, result),
    refreshSettings: () => refreshSettings(shell),
    refreshCommands: () => refreshCommands(shell),
    refreshJobs: () => refreshJobs(shell),
    refreshNodes: () => refreshNodes(shell),
    refreshPlugins: () => refreshPlugins(shell),
    notify: (diagnostic) => notify(shell, diagnostic),
    setMode: (mode) => setMode(shell, mode),
  };
  shell.coreHost = coreHostOf(shell);
  applySettings(shell, emptySettings());
  return shell;
}

/**
 * Starts what keeps the window current: the event stream and its listeners, the route, the OS scheme and the hidden
 * window pause. shell.stop ends the stream.
 * @param {Shell} shell
 */
function startShell(shell) {
  watchRoute(shell.stores.route);
  shell.hub.subscribe(
    { eventTypes: ["job.state_changed"], subjectIds: [] },
    coalesce(() => shell.refreshJobs()),
  );
  shell.hub.subscribe(
    { eventTypes: ["command.finished"], subjectIds: [] },
    coalesce(() => shell.refreshSettings()),
  );
  shell.hub.subscribe({ eventTypes: ["node.heartbeat"], subjectIds: [] }, (event) => keepTelemetry(shell, event));
  shell.hub.subscribe({ eventTypes: ["egress.active"], subjectIds: [] }, (event) => keepEgress(shell, event));
  watchColorScheme(window.matchMedia(DARK_SCHEME_QUERY), () => applySettings(shell, shell.stores.settings.get()));
  document.addEventListener("visibilitychange", () => shell.hub.setHidden(document.hidden));
  shell.hub.start();
}

/**
 * Stores settings.get's result and resolves the display from it: the mode, and the theme for this OS (system follows
 * the OS scheme, with no command). The theme's variables are written to the page.
 * @param {Shell} shell
 * @param {{ values: any, sources: any[] }} result
 */
function applySettings(shell, result) {
  const values = result.values ?? {};
  const theme = themeTokensFor(values, osNow());
  shell.stores.settings.set(result);
  shell.stores.display.set({ mode: globalMode(values), theme });
  applyTheme(theme);
}

/** @returns {{ prefersDark: boolean, prefersReducedMotion: boolean }} */
function osNow() {
  return {
    prefersDark: window.matchMedia(DARK_SCHEME_QUERY).matches,
    prefersReducedMotion: window.matchMedia(REDUCED_MOTION_QUERY).matches,
  };
}

/**
 * @param {Shell} shell
 */
async function refreshSettings(shell) {
  const outcome = await settingsGet(shell.client, {});
  if (outcome.ok) {
    shell.applySettings(outcome.result);
  } else {
    outcome.diagnostics.forEach((diagnostic) => notify(shell, diagnostic));
  }
}

/**
 * The global mode is the ui.mode setting: settings.set writes it, and the refreshed settings change the display.
 * A refusal is shown as its diagnostic.
 * @param {Shell} shell
 * @param {"basic" | "advanced"} mode
 */
async function setMode(shell, mode) {
  const outcome = await settingsSet(shell.client, { key: "ui.mode", value: mode });
  if (outcome.ok) {
    await refreshSettings(shell);
  } else {
    outcome.diagnostics.forEach((diagnostic) => notify(shell, diagnostic));
  }
}

/** @param {Shell} shell */
async function refreshCommands(shell) {
  try {
    shell.stores.commands.set(await listCommands(shell.client));
  } catch (error) {
    notify(
      shell,
      /** @type {any} */ (error)?.cause ?? pluginDiagnostic("RRS-U0001", { address: shell.client.baseUrl }),
    );
  }
}

/** @param {Shell} shell */
async function refreshJobs(shell) {
  const outcome = await jobList(shell.client, {});
  if (outcome.ok) {
    shell.stores.jobs.set(outcome.result.jobs);
  }
}

/** @param {Shell} shell */
async function refreshNodes(shell) {
  const outcome = await nodeList(shell.client, {});
  if (outcome.ok) {
    shell.stores.nodes.set(outcome.result.nodes);
  }
}

/** @param {Shell} shell */
async function refreshPlugins(shell) {
  const outcome = await pluginList(shell.client, {});
  if (outcome.ok) {
    shell.stores.plugins.set(outcome.result.plugins);
  }
}

/**
 * Keeps the last telemetry sample of each node (node.heartbeat carries NodeTelemetry).
 * @param {Shell} shell
 * @param {BusEvent} event
 */
function keepTelemetry(shell, event) {
  // The payload is a oneof; the case names which field holds the sample (contracts 4.3).
  const payload = /** @type {any} */ (event.payload);
  const sample = payload?.case === "nodeHeartbeat" ? payload.value : null;
  if (sample !== null) {
    shell.stores.telemetry.set({ ...shell.stores.telemetry.get(), [event.subjectId]: sample });
  }
}

/**
 * Keeps the egress reports the badge shows; the status bar drops those no longer held.
 * @param {Shell} shell
 * @param {BusEvent} event
 */
function keepEgress(shell, event) {
  const report = egressReportOf(event);
  if (report !== null) {
    const now = Date.now();
    shell.stores.egress.set(heldEgress(recordEgress(shell.stores.egress.get(), report, now), now));
  }
}

/**
 * Adds a Diagnostic to the toast list. The same code replaces its earlier toast.
 * @param {Shell} shell
 * @param {Diagnostic} diagnostic
 */
function notify(shell, diagnostic) {
  // The code is in the text, so a toast says which catalog entry it is, as DiagnosticView does in Advanced mode.
  const message = `${diagnostic.message} (${diagnostic.code})`;
  const toast = { id: diagnostic.code, message, tone: /** @type {const} */ ("error") };
  shell.stores.notices.set(showToast(shell.stores.notices.get(), toast));
}

/**
 * Runs a task once after the current burst of calls: many events in one tick cost one fetch.
 * @param {() => void} task
 * @returns {() => void}
 */
function coalesce(task) {
  let scheduled = false;
  return () => {
    if (scheduled) {
      return;
    }
    scheduled = true;
    setTimeout(() => {
      scheduled = false;
      task();
    }, 0);
  };
}
