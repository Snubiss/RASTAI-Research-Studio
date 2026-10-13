// The window's HostServices and the in-process hosts built from them (layout frontend-and-plugins.md 7.2, 8.1). The
// app builds the services once per window, from its own client, hub, stores and navigation; every host (the core
// sections' and a first-party plugin's) is one createInProcessHost over them. Each service is plain data or a plain
// function: a host never hands out the window, the client or a DOM node.

import { createInProcessHost, CORE_PLUGIN_ID } from "@rastai/plugin";
import { showToast } from "@rastai/ui";

import { navigateToFragment } from "../routing/navigate.js";
import { withoutLockedTokens } from "../state/theme.js";
import { contributionsOf } from "./plugin-registry.js";

/** @typedef {import("@rastai/plugin").PluginHost} PluginHost */
/** @typedef {import("@rastai/plugin").HostServices} HostServices */
/** @typedef {import("../app/shell.js").Shell} Shell */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * The services every host of this window shares, bound to the window's shell.
 * @param {Shell} shell
 * @returns {HostServices}
 */
export function hostServicesOf(shell) {
  return {
    executor: shell.client,
    scopeOf: (commandId) => {
      /** @type {{ commandId: string, scope: string }[]} */
      const commands = shell.stores.commands.get();
      return commands.find((command) => command.commandId === commandId)?.scope ?? "";
    },
    subscribe: (filter, onEvent) => shell.hub.subscribe(filter, onEvent),
    mode: () => shell.stores.display.get().mode,
    // Locked tokens are taken out here, so no host can hand a plugin the security colors (contracts 5.6).
    theme: () => withoutLockedTokens(shell.stores.display.get().theme),
    onDisplayChange: (listener) => shell.stores.display.subscribe(listener),
    navigate: (route) => navigateToFragment(route),
    setContext: (pluginId, values) => {
      const context = shell.stores.context.get();
      shell.stores.context.set({ ...context, [pluginId]: { ...context[pluginId], ...values } });
    },
    contributions: (kind) => contributionsOf(shell.stores.plugins.get(), kind),
    showDiagnostics: (pluginId, diagnostics) =>
      diagnostics.forEach((diagnostic) => showNotice(shell, pluginId, diagnostic)),
  };
}

/**
 * The host of the core sections: the owner's, with no acting plugin id and no scope check (layout 10.2).
 * @param {Shell} shell
 * @returns {PluginHost}
 */
export function coreHostOf(shell) {
  return createInProcessHost(CORE_PLUGIN_ID, [], hostServicesOf(shell));
}

/**
 * Adds a plugin's Diagnostic to the toast list. The same plugin and code replace their earlier toast.
 * @param {Shell} shell
 * @param {string} pluginId
 * @param {Diagnostic} diagnostic
 */
function showNotice(shell, pluginId, diagnostic) {
  const toast = {
    id: `${pluginId}:${diagnostic.code}`,
    message: `${diagnostic.message} (${diagnostic.code})`,
    tone: /** @type {const} */ ("error"),
  };
  shell.stores.notices.set(showToast(shell.stores.notices.get(), toast));
}
