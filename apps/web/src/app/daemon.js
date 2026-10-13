// The window's one DaemonClient (layout frontend-and-plugins.md 8.1, 2.1). It carries the cookie credential and the
// session key kept in memory only. The version check compares this build with the daemon's (RRS-U0004).

import { connectToDaemon } from "@rastai/client";

import { APP_VERSION, CODE_VERSION_DIFFERS } from "../constants.js";
import { pluginDiagnostic } from "@rastai/plugin";

/** @typedef {import("@rastai/client").DaemonClient} DaemonClient */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * Connects to the daemon this page came from, with the window's session key.
 * @param {string} baseUrl    the page's origin
 * @param {string} sessionKey
 * @returns {Promise<{ client: DaemonClient | null, diagnostic: Diagnostic | null }>}
 */
export async function connectDaemon(baseUrl, sessionKey) {
  const result = await connectToDaemon({ baseUrl, credential: { kind: "cookie", sessionKey } });
  if ("execute" in result) {
    return { client: result, diagnostic: null };
  }
  return { client: null, diagnostic: result };
}

/**
 * The problem with the daemon's version, or null when it is this build's. A build with no version (tests) has
 * nothing to compare, so it has no problem.
 * @param {string} daemonVersion
 * @param {string} [appVersion]   this build's version; the window's own by default
 * @returns {Diagnostic | null}
 */
export function versionProblem(daemonVersion, appVersion = APP_VERSION) {
  if (appVersion === "" || daemonVersion === appVersion) {
    return null;
  }
  return pluginDiagnostic(CODE_VERSION_DIFFERS, { app_version: appVersion, daemon_version: daemonVersion });
}
