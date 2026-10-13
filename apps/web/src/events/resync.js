// What the window does when the daemon no longer holds its gap (RRS-C0401, layout frontend-and-plugins.md 2.2): the
// events it missed can't be replayed, so every result it holds may be stale. It drops the query cache and refetches
// the stores from the daemon's list and get commands.

import { invalidateQueries } from "@rastai/plugin";

/** @typedef {import("../app/shell.js").Shell} Shell */

/**
 * @param {Shell} shell
 * @returns {Promise<void>}
 */
export async function resyncShell(shell) {
  invalidateQueries(shell.coreHost);
  await Promise.all([shell.refreshSettings(), shell.refreshCommands(), shell.refreshJobs(), shell.refreshNodes()]);
}
