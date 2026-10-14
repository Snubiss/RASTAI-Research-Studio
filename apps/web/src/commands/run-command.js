// Running a command from the palette (layout frontend-and-plugins.md 2.5 step 6). The command runs with the arguments the
// context filled in; a failure's diagnostics go to the toast list, each with its catalog code, and a success says what
// it did, a job by its id. A command that needs input never runs from here: the palette's form runs it.

import { showToast } from "@rastai/ui";

/** @typedef {import("../app/shell.js").Shell} Shell */

/**
 * @param {Shell} shell
 * @param {{ commandId: string, title: string }} command
 * @param {Object<string, any>} args
 * @returns {Promise<boolean>} whether the command ran
 */
export async function runFromPalette(shell, command, args) {
  const outcome = await shell.coreHost.execute(command.commandId, args);
  if (!outcome.ok) {
    outcome.diagnostics.forEach((diagnostic) => shell.notify(diagnostic));
    return false;
  }
  const message = outcome.jobId ? `${command.title} started job ${outcome.jobId}.` : `${command.title} finished.`;
  shell.stores.notices.set(
    showToast(shell.stores.notices.get(), { id: `palette:${command.commandId}`, message, tone: "success" }),
  );
  return true;
}
