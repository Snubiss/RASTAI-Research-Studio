// The boot steps main.jsx calls, in order (layout frontend-and-plugins.md 8.3): sign in from the login code in the
// fragment, connect, then read the settings, the commands and the plugins. Each step returns what it got, and a
// failure is a Diagnostic, never a throw, so the app can show what went wrong.

import {
  exchangeLoginCode,
  listCommands,
  pluginList,
  readLoginCode,
  removeLoginCodeFromUrl,
  settingsGet,
} from "@rastai/client";
import { pluginDiagnostic } from "@rastai/plugin";

import { CODE_DAEMON_UNREACHABLE } from "../constants.js";

/** @typedef {import("@rastai/client").DaemonClient} DaemonClient */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("@rastai/proto").CommandDescriptor} CommandDescriptor */

/**
 * Signs the window in from the login code in the fragment, then removes the code from the address bar. With no code
 * the window has no session key, and the caller shows the signed-out screen.
 * @param {Location} location
 * @returns {Promise<{ sessionKey: string | null, diagnostic: Diagnostic | null }>}
 */
export async function signIn(location) {
  const code = readLoginCode(location);
  if (code === null) {
    return { sessionKey: null, diagnostic: null };
  }
  const result = await exchangeLoginCode(location.origin, code);
  removeLoginCodeFromUrl();
  return result;
}

/**
 * What a window with no session shows (layout 8.3): RRS-U0001 means the daemon can't be reached, so the offline screen;
 * a rejected or expired code (RRS-U0002), or no code at all, is the signed-out screen with that code.
 * @param {Diagnostic | null} diagnostic   what signIn returned
 * @returns {"offline" | "signed_out"}
 */
export function noSessionScreen(diagnostic) {
  return diagnostic?.code === CODE_DAEMON_UNREACHABLE ? "offline" : "signed_out";
}

/**
 * The settings as the daemon's effective values, with each key's layer.
 * @param {DaemonClient} client
 * @returns {Promise<{ result: any, diagnostic: Diagnostic | null }>}
 */
export async function loadSettings(client) {
  const outcome = await settingsGet(client, {});
  return { result: outcome.ok ? outcome.result : null, diagnostic: firstDiagnostic(outcome) };
}

/**
 * The registered commands of every section (ListCommands). A failed call is an empty list and a Diagnostic.
 * @param {DaemonClient} client
 * @returns {Promise<{ commands: CommandDescriptor[], diagnostic: Diagnostic | null }>}
 */
export async function loadCommands(client) {
  try {
    return { commands: await listCommands(client), diagnostic: null };
  } catch (error) {
    // listCommands keeps the client's own Diagnostic as the error's cause; a plain failure has none.
    const cause = /** @type {any} */ (error)?.cause ?? null;
    return { commands: [], diagnostic: cause ?? pluginDiagnostic("RRS-U0001", { address: client.baseUrl }) };
  }
}

/**
 * The installed plugins (plugin.list).
 * @param {DaemonClient} client
 * @returns {Promise<{ plugins: any[], diagnostic: Diagnostic | null }>}
 */
export async function loadPlugins(client) {
  const outcome = await pluginList(client, {});
  return { plugins: outcome.ok ? (outcome.result?.plugins ?? []) : [], diagnostic: firstDiagnostic(outcome) };
}

/**
 * @param {{ ok: boolean, diagnostics: Diagnostic[] }} outcome
 * @returns {Diagnostic | null}
 */
export function firstDiagnostic(outcome) {
  if (outcome.ok) {
    return null;
  }
  return outcome.diagnostics[0] ?? null;
}
