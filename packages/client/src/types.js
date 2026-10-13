// The client's JSDoc types (layout frontend-and-plugins.md 3.2). JSDoc only: nothing here runs.

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("./command-executor.js").ExecuteFunction} ExecuteFunction */

// The connect-es clients of the two daemon services. Other files call them with plain request objects, so only
// connection.js imports the library itself.
/** @typedef {import("@connectrpc/connect").Client<typeof import("@rastai/proto").CommandService>} CommandServiceClient */
/** @typedef {import("@connectrpc/connect").Client<typeof import("@rastai/proto").EventService>} EventServiceClient */

/**
 * How the client proves who it is. The browser app always uses the cookie.
 * @typedef {object} Credential
 * @property {"cookie" | "bearer"} kind
 * @property {string} [sessionKey]   only for "cookie": the key exchangeLoginCode returned, held in memory only
 * @property {string} [token]        only for "bearer": a scoped token from token.create
 */

/**
 * @typedef {object} ConnectOptions
 * @property {string} baseUrl           e.g. location.origin
 * @property {Credential} credential
 * @property {boolean} [useJsonWire]    Connect JSON instead of binary for commands, for devtools (?wire=json)
 */

/**
 * One connection to one daemon. It satisfies CommandExecutor through its execute field.
 * @typedef {object} DaemonClient
 * @property {string} baseUrl
 * @property {string} daemonVersion
 * @property {Credential} credential
 * @property {CommandServiceClient} commands
 * @property {EventServiceClient} events
 * @property {ExecuteFunction} execute
 */

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
