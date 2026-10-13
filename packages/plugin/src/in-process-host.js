// The PluginHost for code that runs inside the app: the core sections and the first-party plugins (layout
// frontend-and-plugins.md 7.2). The app builds one HostServices per window at boot (apps/web/src/plugin-host/:
// its DaemonClient, event hub, display settings and registries) and one host per plugin from it. The host adds
// what a plugin may not choose for itself: its acting plugin id on every call and the check of its
// commands:[scopes] permission before the call leaves the window (contracts 6.3; the daemon checks it again).

import { CODE_SCOPE_NOT_GRANTED, pluginDiagnostic, refusedOutcome } from "./plugin-diagnostic.js";

/** @typedef {import("./host-api.js").PluginHost} PluginHost */
/** @typedef {import("./host-api.js").EventFilter} EventFilter */
/** @typedef {import("./host-api.js").Contribution} Contribution */
/** @typedef {import("./host-api.js").BusEvent} BusEvent */
/** @typedef {import("./host-api.js").Diagnostic} Diagnostic */
/** @typedef {import("./host-api.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("./host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("./host-api.js").UiMode} UiMode */
/** @typedef {import("./host-api.js").ThemeTokens} ThemeTokens */
/** @typedef {import("@rastai/client").CommandExecutor} CommandExecutor */

/**
 * The pseudo plugin id of the app's own sections (layout 10.2). Their calls run as the owner, so they carry no
 * acting plugin id and no scope check.
 */
export const CORE_PLUGIN_ID = "core";
// A third-party plugin always holds its own scope, written like this wherever a scope is text (contracts 6.3).
const OWN_SCOPE_PREFIX = "plugin:";

/**
 * What the app shares with every in-process host of one window. Each field is a plain function or object the app
 * already has; the host only binds them to one plugin.
 * @typedef {object} HostServices
 * @property {CommandExecutor} executor        the window's DaemonClient
 * @property {(commandId: string) => string} scopeOf   the scope the command index lists; "" for a command it lacks
 * @property {(filter: EventFilter, onEvent: (event: BusEvent) => void) => (() => void)} subscribe   the event hub's
 * @property {() => UiMode} mode
 * @property {() => ThemeTokens} theme         resolved tokens, locked ones excluded
 * @property {(listener: () => void) => (() => void)} onDisplayChange
 * @property {(route: string) => void} navigate
 * @property {(pluginId: string, values: Object<string, string>) => void} setContext
 * @property {(kind: string) => Contribution[]} contributions   every enabled plugin's entries of that kind
 * @property {(pluginId: string, diagnostics: Diagnostic[]) => void} showDiagnostics
 */

/**
 * @param {string} pluginId              CORE_PLUGIN_ID for the app's own sections
 * @param {string[]} grantedScopes       the plugin's granted commands:[scopes]; ignored for core
 * @param {HostServices} services
 * @returns {PluginHost}
 */
export function createInProcessHost(pluginId, grantedScopes, services) {
  return {
    pluginId,
    execute: (commandId, args, options) =>
      executeForPlugin(pluginId, grantedScopes, services, commandId, args, options),
    subscribe: services.subscribe,
    mode: services.mode,
    theme: services.theme,
    onDisplayChange: services.onDisplayChange,
    navigate: services.navigate,
    setContext: (values) => services.setContext(pluginId, values),
    contributions: services.contributions,
    showDiagnostics: (diagnostics) => services.showDiagnostics(pluginId, diagnostics),
  };
}

/**
 * Runs one command for a plugin. A call outside the plugin's scopes is refused here, as an outcome, never a throw.
 * @param {string} pluginId
 * @param {string[]} grantedScopes
 * @param {HostServices} services
 * @param {string} commandId
 * @param {object} args
 * @param {ExecuteOptions} [options]
 * @returns {Promise<ExecuteOutcome>}
 */
async function executeForPlugin(pluginId, grantedScopes, services, commandId, args, options) {
  const scope = services.scopeOf(commandId);
  if (!mayRun(pluginId, grantedScopes, scope)) {
    const values = { command_id: commandId, scope, principal: OWN_SCOPE_PREFIX + pluginId };
    return refusedOutcome(pluginDiagnostic(CODE_SCOPE_NOT_GRANTED, values));
  }
  return services.executor.execute(commandId, args, hostOptions(pluginId, options));
}

/**
 * Whether a plugin may run a command that needs this scope. A command the index doesn't list ("") goes on to the
 * daemon, which refuses an unknown command and checks the scope itself, narrowed by the acting plugin id.
 * @param {string} pluginId
 * @param {string[]} grantedScopes
 * @param {string} scope
 * @returns {boolean}
 */
export function mayRun(pluginId, grantedScopes, scope) {
  if (pluginId === CORE_PLUGIN_ID || scope === "") {
    return true;
  }
  return scope === OWN_SCOPE_PREFIX + pluginId || grantedScopes.includes(scope);
}

/**
 * The options that reach the executor. A plugin may set only the project and dry run; the acting plugin id is
 * always the host's own (empty for core), whatever the caller passed.
 * @param {string} pluginId
 * @param {ExecuteOptions} [options]
 * @returns {ExecuteOptions}
 */
export function hostOptions(pluginId, options) {
  return {
    projectId: options?.projectId ?? "",
    dryRun: options?.dryRun ?? false,
    actingPluginId: pluginId === CORE_PLUGIN_ID ? "" : pluginId,
  };
}
