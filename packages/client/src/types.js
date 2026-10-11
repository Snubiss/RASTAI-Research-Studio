// The client's JSDoc types (layout frontend-and-plugins.md 3.2). JSDoc only: nothing here runs. For now only the
// two that command-executor.js names; Credential, ConnectOptions and DaemonClient join with the connection (M1-50).

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * @typedef {object} ExecuteOptions
 * @property {string} [projectId]
 * @property {boolean} [dryRun]
 * @property {string} [actingPluginId]   set only by plugin hosts; the daemon narrows the call to the plugin's grant
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
