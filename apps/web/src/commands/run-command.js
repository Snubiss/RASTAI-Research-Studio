// Running a command from the palette (layout frontend-and-plugins.md 2.5 step 6). The command runs with the arguments the
// context filled in; a failure's diagnostics go to the toast list, each with its catalog code, and a success says what
// it did, a job by its id. A command that needs input never runs from here: the palette's form runs it, through
// paletteHostOf, so the redirect below holds for the form too.

import { showToast } from "@rastai/ui";

import { navigateTo } from "../routing/navigate.js";
import { sectionRoute } from "../routing/route.js";

/** @typedef {import("../app/shell.js").Shell} Shell */
/** @typedef {import("@rastai/plugin").PluginHost} PluginHost */

// A token is made only behind the locked TokenConfirm prompt on Settings > API tokens (layout 8.2). The palette and
// the default shortcuts would make one with no prompt and throw the value away, so they send the person there instead.
const REDIRECTED_COMMAND_IDS = ["token.create"];
const SETTINGS_SECTION_ID = "settings";

/**
 * Whether the palette and the shortcuts send this command to Settings instead of running it.
 * @param {string} commandId
 * @returns {boolean}
 */
export function isRedirectedCommand(commandId) {
  return REDIRECTED_COMMAND_IDS.includes(commandId);
}

/**
 * @param {Shell} shell
 * @param {{ commandId: string, title: string }} command
 * @param {Object<string, any>} args
 * @returns {Promise<boolean>} whether the command ran
 */
export async function runFromPalette(shell, command, args) {
  if (isRedirectedCommand(command.commandId)) {
    openSettings(shell);
    return false;
  }
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

/**
 * The host the palette's forms run through (CommandForm's Run calls its execute). A redirected command is sent to
 * Settings there too, so the form cannot make a token the palette's own Run would not make.
 * @param {Shell} shell
 * @returns {PluginHost}
 */
export function paletteHostOf(shell) {
  const coreHost = shell.coreHost;
  return {
    ...coreHost,
    execute: (commandId, args, options) => {
      if (isRedirectedCommand(commandId)) {
        openSettings(shell);
        return Promise.resolve({ ok: false, result: null, jobId: null, diagnostics: [], journalId: null });
      }
      return coreHost.execute(commandId, args, options);
    },
  };
}

/**
 * Closes the palette and opens Settings, where a redirected command's own locked step is.
 * @param {Shell} shell
 */
function openSettings(shell) {
  shell.stores.palette.set({ open: false, query: "" });
  navigateTo(sectionRoute(SETTINGS_SECTION_ID));
}
