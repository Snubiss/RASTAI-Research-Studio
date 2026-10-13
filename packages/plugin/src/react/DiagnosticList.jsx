// The diagnostics of one outcome, each shown by @rastai/ui's DiagnosticView. A fix button runs its fix command
// through the panel's host, and the fix's own failure goes to the shell's diagnostics (layout 2.7).

import { DiagnosticView } from "@rastai/ui";

import { pluginDiagnostic } from "../plugin-diagnostic.js";
import { usePluginHost } from "./use-plugin-host.js";

/** @typedef {import("../host-api.js").Diagnostic} Diagnostic */
/** @typedef {import("../host-api.js").PluginHost} PluginHost */

// Invalid command arguments (contracts 10): a fix whose fix_arguments_json isn't a JSON object can't be run.
const CODE_INVALID_ARGUMENTS = "RRS-C0002";
// How much of the bad text the Diagnostic quotes; enough to recognize it, short enough for a toast.
const QUOTED_ARGUMENTS_LENGTH = 80;

/**
 * @param {object} props
 * @param {Diagnostic[]} props.diagnostics
 */
export function DiagnosticList({ diagnostics }) {
  const host = usePluginHost();
  return (
    <>
      {diagnostics.map((diagnostic, index) => (
        <DiagnosticView
          key={`${diagnostic.code}-${index}`}
          diagnostic={diagnostic}
          onFix={(commandId, argumentsJson) => runFix(host, commandId, argumentsJson)}
        />
      ))}
    </>
  );
}

/**
 * @param {PluginHost} host
 * @param {string} commandId
 * @param {string} argumentsJson   the daemon's fix_arguments_json; empty for a fix without arguments
 */
async function runFix(host, commandId, argumentsJson) {
  const fixArguments = parseFixArguments(argumentsJson);
  if (fixArguments === null) {
    const found = argumentsJson.slice(0, QUOTED_ARGUMENTS_LENGTH);
    host.showDiagnostics([pluginDiagnostic(CODE_INVALID_ARGUMENTS, { found, expected: "a JSON object" })]);
    return;
  }
  const outcome = await host.execute(commandId, fixArguments);
  if (!outcome.ok) {
    host.showDiagnostics(outcome.diagnostics);
  }
}

/**
 * The fix's arguments, or null when the daemon sent text that isn't a JSON object. A click handler that threw
 * would leave an unhandled rejection and no word to the user.
 * @param {string} argumentsJson
 * @returns {object | null}
 */
function parseFixArguments(argumentsJson) {
  if (argumentsJson === "") {
    return {};
  }
  let parsed;
  try {
    parsed = JSON.parse(argumentsJson);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }
  return parsed;
}
