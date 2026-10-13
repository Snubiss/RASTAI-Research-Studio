// Running a command from a button or a form (layout frontend-and-plugins.md 7.2): the run function, whether a run
// is in flight, and the last outcome. A failed command is an outcome with diagnostics, never a thrown error.

import { useCallback, useState } from "react";

import { usePluginHost } from "./use-plugin-host.js";

/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */

/**
 * @typedef {object} CommandState
 * @property {(args: object) => Promise<ExecuteOutcome>} run
 * @property {boolean} running
 * @property {ExecuteOutcome | null} outcome   the last one
 */

/**
 * @param {string} commandId
 * @returns {CommandState}
 */
export function useCommand(commandId) {
  const host = usePluginHost();
  // A count, not a flag, so the first of two overlapping runs to finish doesn't report the other as done.
  const [runsInFlight, setRunsInFlight] = useState(0);
  const [outcome, setOutcome] = useState(/** @type {ExecuteOutcome | null} */ (null));
  const run = useCallback(
    /** @param {object} args */
    async (args) => {
      setRunsInFlight((count) => count + 1);
      const result = await host.execute(commandId, args);
      setOutcome(result);
      setRunsInFlight((count) => count - 1);
      return result;
    },
    [host, commandId],
  );
  return { run, running: runsInFlight > 0, outcome };
}
