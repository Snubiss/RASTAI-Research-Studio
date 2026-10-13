// A PluginHost for tests and `rastai plugin test` (layout frontend-and-plugins.md 7.2): canned command outcomes,
// recorded calls and a small in-memory world (events, display, routes, contributions) the test drives. It is the
// in-process host over that world, so a panel tested here meets the same acting-id and option rules as in the app.
// The host and the test's view of it are two fields of one plain object: the panel gets `host`, the test reads
// the rest.

import { builtInTheme } from "@rastai/ui";

import { eventMatches } from "../event-filter.js";
import { createInProcessHost } from "../in-process-host.js";
import { CODE_UNKNOWN_COMMAND, pluginDiagnostic, refusedOutcome } from "../plugin-diagnostic.js";

/** @typedef {import("../host-api.js").PluginHost} PluginHost */
/** @typedef {import("../host-api.js").EventFilter} EventFilter */
/** @typedef {import("../host-api.js").Contribution} Contribution */
/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("../host-api.js").Diagnostic} Diagnostic */
/** @typedef {import("../host-api.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("../host-api.js").UiMode} UiMode */
/** @typedef {import("../host-api.js").ThemeTokens} ThemeTokens */
/** @typedef {import("../in-process-host.js").HostServices} HostServices */

// ui.mode's prototype default (contracts 11.2) and the light built-in theme, as a fresh app window has them.
/** @type {UiMode} */
const DEFAULT_MODE = "advanced";
/** @type {import("@rastai/ui").ThemeOptions} */
const DEFAULT_THEME_OPTIONS = { density: "comfortable", reducedMotion: false, colorblind: false };

/**
 * One execute the host passed on, with the options it sent (the acting plugin id already filled in).
 * @typedef {object} RecordedCall
 * @property {string} commandId
 * @property {object} args
 * @property {ExecuteOptions} options
 */

/**
 * A PluginHost with canned outcomes, plus what the test needs to drive and look at.
 * @typedef {object} MockHost
 * @property {PluginHost} host
 * @property {RecordedCall[]} calls                       every execute that reached the canned outcomes, in order
 * @property {(event: BusEvent) => void} emit             delivers a fake event to the matching subscribers
 * @property {(mode: UiMode, theme: ThemeTokens) => void} setDisplay   changes the display and tells the listeners
 * @property {string[]} routes                            every navigate, in order
 * @property {Object<string, string>[]} contexts          every setContext, in order
 * @property {Diagnostic[][]} shownDiagnostics            every showDiagnostics, in order
 * @property {Object<string, Contribution[]>} contributions   what host.contributions(kind) returns; tests fill it
 */

/**
 * @param {string} pluginId
 * @param {Object<string, (args: object) => ExecuteOutcome>} outcomes   command id -> canned outcome
 * @returns {MockHost}
 */
export function createMockHost(pluginId, outcomes) {
  /** @type {{ filter: EventFilter, onEvent: (event: BusEvent) => void }[]} */
  const subscribers = [];
  /** @type {Set<() => void>} */
  const displayListeners = new Set();
  const display = { mode: DEFAULT_MODE, theme: builtInTheme("light", DEFAULT_THEME_OPTIONS) };
  /** @type {RecordedCall[]} */
  const calls = [];
  /** @type {string[]} */
  const routes = [];
  /** @type {Object<string, string>[]} */
  const contexts = [];
  /** @type {Diagnostic[][]} */
  const shownDiagnostics = [];
  /** @type {Object<string, Contribution[]>} */
  const contributions = {};

  /** @type {HostServices} */
  const services = {
    executor: {
      execute: async (commandId, args, options) => {
        calls.push({ commandId, args, options: options ?? {} });
        return cannedOutcome(outcomes, commandId, args);
      },
    },
    // The mock grants every scope: a panel test is about the panel, not the plugin's permission.
    scopeOf: () => "",
    subscribe: (filter, onEvent) => {
      const subscriber = { filter, onEvent };
      subscribers.push(subscriber);
      return () => removeItem(subscribers, subscriber);
    },
    mode: () => display.mode,
    theme: () => display.theme,
    onDisplayChange: (listener) => {
      displayListeners.add(listener);
      return () => displayListeners.delete(listener);
    },
    navigate: (route) => routes.push(route),
    setContext: (_pluginId, values) => contexts.push(values),
    contributions: (kind) => contributions[kind] ?? [],
    showDiagnostics: (_pluginId, diagnostics) => shownDiagnostics.push(diagnostics),
  };

  return {
    host: createInProcessHost(pluginId, [], services),
    calls,
    emit: (event) => {
      // A copy, so a subscriber that unsubscribes while handling the event doesn't make the loop skip the next one.
      for (const subscriber of [...subscribers]) {
        if (eventMatches(subscriber.filter, event)) {
          subscriber.onEvent(event);
        }
      }
    },
    setDisplay: (mode, theme) => {
      display.mode = mode;
      display.theme = theme;
      displayListeners.forEach((listener) => listener());
    },
    routes,
    contexts,
    shownDiagnostics,
    contributions,
  };
}

/**
 * The canned outcome for a command. A command without one is unknown, as the daemon would say.
 * @param {Object<string, (args: object) => ExecuteOutcome>} outcomes
 * @param {string} commandId
 * @param {object} args
 * @returns {ExecuteOutcome}
 */
function cannedOutcome(outcomes, commandId, args) {
  const outcomeFor = outcomes[commandId];
  if (!outcomeFor) {
    return refusedOutcome(pluginDiagnostic(CODE_UNKNOWN_COMMAND, { command_id: commandId }));
  }
  return outcomeFor(args);
}

/**
 * @param {any[]} list
 * @param {any} item
 */
function removeItem(list, item) {
  const index = list.indexOf(item);
  if (index >= 0) {
    list.splice(index, 1);
  }
}
