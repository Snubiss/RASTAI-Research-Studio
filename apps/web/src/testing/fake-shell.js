// Test support for the app's browser tests: a fake DaemonClient that answers the commands the shell asks for, a fake
// event stream the test drives by hand, and a shell built over both. Nothing here is part of the app.

import { pluginDiagnostic, registeredPluginUi } from "@rastai/plugin";

import { createShell } from "../app/shell.js";
import { registerShellUis } from "../plugin-host/load-plugins.js";

/** @typedef {import("../app/shell.js").Shell} Shell */

export const FAKE_BASE_URL = "http://127.0.0.1:47820";

/**
 * A DaemonClient with answers per command. An answer is an outcome, or a function from the arguments to one; a command
 * with no answer is unknown, as the daemon would say.
 * @param {{ answers?: Object<string, any>, daemonVersion?: string }} [options]
 */
export function fakeClient(options = {}) {
  /** @type {{ commandId: string, args: any, options: any }[]} */
  const calls = [];
  const answers = options.answers ?? {};
  return {
    baseUrl: FAKE_BASE_URL,
    daemonVersion: options.daemonVersion ?? "0.1.0",
    credential: { kind: "cookie", sessionKey: "test-key" },
    calls,
    /** @param {string} commandId @param {any} args @param {any} executeOptions */
    execute: async (commandId, args, executeOptions) => {
      calls.push({ commandId, args, options: executeOptions ?? {} });
      const answer = answers[commandId];
      if (answer === undefined) {
        return refused("RRS-C0001", commandId);
      }
      return typeof answer === "function" ? answer(args) : answer;
    },
  };
}

/**
 * An outcome that refused a command with a catalog code.
 * @param {string} code
 * @param {string} commandId
 * @returns {any}
 */
export function refused(code, commandId) {
  return {
    ok: false,
    result: null,
    jobId: null,
    diagnostics: [pluginDiagnostic(code, { command_id: commandId })],
    journalId: null,
  };
}

/**
 * A successful outcome.
 * @param {any} result
 * @returns {any}
 */
export function answered(result) {
  return { ok: true, result, jobId: null, diagnostics: [], journalId: null };
}

/**
 * The event stream the shell opens, driven by the test: `emit` delivers an event, `setStatus` reports the connection.
 */
export function fakeStream() {
  /** @type {{ eventTypes: string[], afterBusSequence: bigint, handlers: any }[]} */
  const opened = [];
  return {
    opened,
    /** @param {any} _client @param {string[]} eventTypes @param {bigint} afterBusSequence @param {any} handlers */
    openStream: (_client, eventTypes, afterBusSequence, handlers) => {
      opened.push({ eventTypes, afterBusSequence, handlers });
      // The client's EventStream shape; the hub keeps the sequence itself and doesn't ask the stream for it.
      return { stop: () => {}, lastBusSequence: () => afterBusSequence };
    },
    /** @param {any} event */
    emit: (event) => {
      opened.at(-1)?.handlers.onEvent(event);
    },
    /** @param {string} status @param {any} [problem] */
    setStatus: (status, problem = null) => {
      opened.at(-1)?.handlers.onStatus(status, problem);
    },
  };
}

/**
 * A bus event as the hub delivers it.
 * @param {string} eventType
 * @param {string} subjectId
 * @param {any} [payload]
 */
export function busEvent(eventType, subjectId, payload = undefined) {
  return { busSequence: 1n, eventType, subjectId, source: "daemon", payload };
}

/**
 * Registers the core UI once per page, so the sections render (a test file may build several shells).
 */
export async function ensureCoreUi() {
  if (registeredPluginUi("core") === null) {
    await registerShellUis([]);
  }
}

/**
 * A shell over a fake client and stream, with settings, commands and plugins set as the daemon would answer them.
 * @param {{ answers?: Object<string, any>, settings?: Object<string, any>, commands?: any[], plugins?: any[] }} [setup]
 * @returns {Promise<{ shell: Shell, client: any, stream: ReturnType<typeof fakeStream> }>}
 */
export async function testShell(setup = {}) {
  await ensureCoreUi();
  const client = fakeClient({ answers: setup.answers ?? {} });
  const stream = fakeStream();
  const shell = createShell(/** @type {any} */ (client), { openStream: stream.openStream });
  shell.applySettings({ values: setup.settings ?? {}, sources: [] });
  shell.stores.commands.set(setup.commands ?? []);
  shell.stores.plugins.set(setup.plugins ?? []);
  shell.loadedPluginIds = ["core"];
  shell.start();
  return { shell, client, stream };
}
