// execute and listCommands: CommandService calls, and ExecuteResponse turned into an ExecuteOutcome (layout
// frontend-and-plugins.md 3.2). A command that fails is an outcome with ok false, never a thrown error.

import { EXECUTE_ATTEMPTS, EXECUTE_RETRY_DELAY_MS } from "./constants.js";
import { callFailureDiagnostic, daemonDiagnosticOf } from "./diagnostics.js";

/** @typedef {import("@rastai/proto").CommandDescriptor} CommandDescriptor */
/** @typedef {import("@rastai/proto").ExecuteRequest} ExecuteRequest */
/** @typedef {import("@rastai/proto").ExecuteResponse} ExecuteResponse */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */
/** @typedef {import("./types.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("./types.js").ExecuteOutcome} ExecuteOutcome */

/**
 * Runs one command. Never throws for an expected failure: a command the daemon refused comes back with its
 * diagnostics, and a transport problem as an outcome with an RRS-U diagnostic. Each call gets a fresh idempotency
 * key; a call that never got an answer is sent once more with the same key, so the daemon doesn't run it twice.
 * Throws only for an answer that breaks the wire contract (a result_json that isn't JSON): a daemon bug.
 * @param {DaemonClient} client
 * @param {string} commandId
 * @param {object} args
 * @param {ExecuteOptions} [options]
 * @returns {Promise<ExecuteOutcome>}
 */
export async function execute(client, commandId, args, options) {
  const request = {
    commandId,
    argumentsJson: JSON.stringify(args ?? {}),
    projectId: options?.projectId ?? "",
    idempotencyKey: crypto.randomUUID(),
    dryRun: options?.dryRun ?? false,
    actingPluginId: options?.actingPluginId ?? "",
  };
  const reply = await sendExecute(client, request);
  if (reply.response === null) {
    return failedOutcome(callFailureDiagnostic(reply.error, client.baseUrl));
  }
  // Read after the retry loop: an answer that came is never sent again, whatever it holds.
  return outcomeFromResponse(commandId, reply.response);
}

/**
 * Sends one Execute, and once more with the same request when no answer came.
 * @param {DaemonClient} client
 * @param {ExecuteRequest | object} request
 * @returns {Promise<{ response: ExecuteResponse | null, error: any }>}   error is the last failure when no answer came
 */
async function sendExecute(client, request) {
  let lastError = null;
  for (let attempt = 1; attempt <= EXECUTE_ATTEMPTS; attempt += 1) {
    try {
      const response = await client.commands.execute(request);
      return { response, error: null };
    } catch (error) {
      lastError = error;
      // The daemon answered with a refusal: sending the same request again gets the same answer.
      if (daemonDiagnosticOf(error) !== null) {
        break;
      }
    }
    if (attempt < EXECUTE_ATTEMPTS) {
      await waitMs(EXECUTE_RETRY_DELAY_MS);
    }
  }
  return { response: null, error: lastError };
}

/**
 * The daemon's command registry, every section. Rejects on a transport failure with an Error whose `cause` is the
 * RRS-U (or the daemon's) Diagnostic, so the caller can show it.
 * @param {DaemonClient} client
 * @returns {Promise<CommandDescriptor[]>}
 */
export async function listCommands(client) {
  try {
    const response = await client.commands.listCommands({ section: "" });
    return response.commands;
  } catch (error) {
    const diagnostic = callFailureDiagnostic(error, client.baseUrl);
    throw new Error(diagnostic.message, { cause: diagnostic });
  }
}

/**
 * @param {string} commandId
 * @param {ExecuteResponse} response
 * @returns {ExecuteOutcome}
 */
function outcomeFromResponse(commandId, response) {
  // result_json is "" when a command has no result or failed; the outcome says null for both.
  const hasResult = response.ok && response.resultJson !== "";
  return {
    ok: response.ok,
    result: hasResult ? parseResult(commandId, response.resultJson) : null,
    jobId: response.jobId === "" ? null : response.jobId,
    diagnostics: response.diagnostics,
    journalId: response.journalId === "" ? null : response.journalId,
  };
}

/**
 * @param {string} commandId
 * @param {string} resultJson
 * @returns {any}
 */
function parseResult(commandId, resultJson) {
  try {
    return JSON.parse(resultJson);
  } catch (error) {
    throw new Error(`The daemon's result_json for ${commandId} is not JSON.`, { cause: error });
  }
}

/**
 * @param {import("@rastai/proto").Diagnostic} diagnostic
 * @returns {ExecuteOutcome}
 */
function failedOutcome(diagnostic) {
  return { ok: false, result: null, jobId: null, diagnostics: [diagnostic], journalId: null };
}

/** @param {number} milliseconds */
function waitMs(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
