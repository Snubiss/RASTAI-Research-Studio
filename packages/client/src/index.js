// @rastai/client: the thin daemon client (layout frontend-and-plugins.md 3). Public exports only. The typed command
// wrappers are re-exported here too, and also stand alone as "@rastai/client/commands", so @rastai/plugin can pass
// them on without pulling the transport into an iframe bundle.

export { connectToDaemon, checkHealth } from "./connection.js";
export { readLoginCode, removeLoginCodeFromUrl, exchangeLoginCode } from "./login.js";
export { execute, listCommands } from "./execute.js";
export { openEventStream } from "./event-stream.js";
export { blobUrl, uploadBlob, readBlobBytes } from "./blobs.js";
export { clientDiagnostic } from "./diagnostics.js";
export { MAX_BROWSER_UPLOAD_BYTES } from "./constants.js";
export * from "./generated/commands.js";

/** @typedef {import("./types.js").Credential} Credential */
/** @typedef {import("./types.js").ConnectOptions} ConnectOptions */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */
/** @typedef {import("./types.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("./types.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("./command-executor.js").CommandExecutor} CommandExecutor */
/** @typedef {import("./command-executor.js").ExecuteFunction} ExecuteFunction */
/** @typedef {import("./event-stream.js").EventStream} EventStream */
/** @typedef {import("./event-stream.js").EventStreamStatus} EventStreamStatus */
/** @typedef {import("./event-stream.js").EventStreamHandlers} EventStreamHandlers */
