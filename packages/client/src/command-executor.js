// The one object shape every caller of a command satisfies (contracts 12.4, owner decision N5): DaemonClient,
// each PluginHost and the mock host. The generated wrappers in generated/commands.js take any CommandExecutor, so
// the same call works in the app, in a plugin panel and in a script. JSDoc only: nothing here runs.

/** @typedef {import("./types.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("./types.js").ExecuteOutcome} ExecuteOutcome */

/**
 * Runs one command. DaemonClient and PluginHost both provide it.
 * @callback ExecuteFunction
 * @param {string} commandId
 * @param {object} args
 * @param {ExecuteOptions} [options]
 * @returns {Promise<ExecuteOutcome>}
 */

/**
 * @typedef {object} CommandExecutor
 * @property {ExecuteFunction} execute
 */

export {};
