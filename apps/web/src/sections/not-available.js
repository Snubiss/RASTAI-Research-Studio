// The "not available yet" state of a section or a command this version doesn't have (catalog RRS-C0010): the one
// honest answer where a screen needs something that isn't built. Built from the catalog, as the plugin SDK builds its
// own Diagnostics, so the text is the catalog's.

import { pluginDiagnostic } from "@rastai/plugin";

const CODE_NOT_AVAILABLE = "RRS-C0010";

/**
 * @param {string} commandId   the command, or a section's name, that isn't available yet
 */
export function notAvailableDiagnostic(commandId) {
  return pluginDiagnostic(CODE_NOT_AVAILABLE, { command_id: commandId });
}
