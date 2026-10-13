// The diagnostics of one outcome, each shown by @rastai/ui's DiagnosticView. A fix button runs its fix command
// through the panel's host, and the fix's own failure goes to the shell's diagnostics (layout 2.7).

import { DiagnosticView } from "@rastai/ui";

import { usePluginHost } from "./use-plugin-host.js";

/** @typedef {import("../host-api.js").Diagnostic} Diagnostic */
/** @typedef {import("../host-api.js").PluginHost} PluginHost */

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
  const outcome = await host.execute(commandId, argumentsJson === "" ? {} : JSON.parse(argumentsJson));
  if (!outcome.ok) {
    host.showDiagnostics(outcome.diagnostics);
  }
}
