// The client's JSDoc types (layout frontend-and-plugins.md 3.2). JSDoc only: nothing here runs. For now only the
// two that command-executor.js names; Credential, ConnectOptions and DaemonClient join with the connection (M1-50).

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * @typedef {object} ExecuteOptions
 * @property {string} [projectId]
 * @property {boolean} [dryRun]
 * @property {string} [actingPluginId]   the owner's only, sent by a plugin UI's bridge: narrows the call to the
 *                                       plugin's grant; any other caller is refused (RRS-C1005)
 */

/**
 * @typedef {object} ExecuteOutcome
 * @property {boolean} ok
 * @property {any} result                parsed result_json; null when ok is false
 * @property {string | null} jobId
 * @property {Diagnostic[]} diagnostics
 * @property {string | null} journalId
 */

export {};
