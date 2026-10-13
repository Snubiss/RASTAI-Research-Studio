// Both shared tables against both PluginHosts M1 has (layout frontend-and-plugins.md 7.5): @rastai/client's
// executor contract (EXECUTOR_CONTRACT_CASES) and this package's host contract (HOST_CONTRACT_CASES), each against
// the mock host and against the in-process host. The in-process host runs over a real DaemonClient on the client's
// fake daemon, so its executor cases go over the wire as a panel's calls do in the app.

import { connectToDaemon } from "@rastai/client";
import { EXECUTOR_CONTRACT_CASES, summarizeOutcome } from "@rastai/client/testing";
import { createFakeDaemon } from "@rastai/client/testing/fake-daemon";
import { builtInTheme } from "@rastai/ui";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { eventMatches } from "../event-filter.js";
import { CORE_PLUGIN_ID, createInProcessHost } from "../in-process-host.js";
import { pluginDiagnostic } from "../plugin-diagnostic.js";
import { HOST_CONTRACT_CASES, HOST_CONTRACT_PLUGIN_ID } from "./host-contract.js";
import { createMockHost } from "./mock-host.js";

/** @typedef {import("@rastai/client").DaemonClient} DaemonClient */
/** @typedef {import("@rastai/client/testing").ExecutorContractCase} ExecutorContractCase */
/** @typedef {import("@rastai/client/testing/fake-daemon").FakeDaemon} FakeDaemon */
/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("../host-api.js").EventFilter} EventFilter */
/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("../in-process-host.js").HostServices} HostServices */
/** @typedef {import("./mock-host.js").MockHost} MockHost */

const SESSION_REFUSED_CODE = "RRS-C1002";
const UNREACHABLE_CODE = "RRS-U0001";
const SIGNED_OUT_CODE = "RRS-U0003";
const LIGHT_THEME = builtInTheme("light", { density: "comfortable", reducedMotion: false, colorblind: false });
// The executor cases' commands and the scope the command index lists for each (contracts 6.4).
/** @type {Object<string, string>} */
const COMMAND_SCOPES = { "job.list": "read", "settings.get": "system.admin", "settings.set": "system.admin" };

/** @type {FakeDaemon} */
let daemon;

beforeEach(() => {
  daemon = createFakeDaemon();
  vi.stubGlobal("fetch", daemon.fetch);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** @returns {Promise<DaemonClient>} */
async function connect() {
  const client = await connectToDaemon({ baseUrl: daemon.baseUrl, credential: { kind: "cookie", sessionKey: "k" } });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  return client;
}

/**
 * An in-process host the way the app builds one: over the window's DaemonClient, with an event hub, a display and
 * the shell's records kept in plain fields. It has the MockHost shape, so the host contract reads both alike.
 * @param {string} pluginId
 * @param {string[]} grantedScopes
 * @param {DaemonClient} client
 * @returns {MockHost}
 */
function inProcessTarget(pluginId, grantedScopes, client) {
  /** @type {{ filter: EventFilter, onEvent: (event: BusEvent) => void }[]} */
  let subscribers = [];
  /** @type {(() => void)[]} */
  let displayListeners = [];
  const display = { mode: /** @type {import("@rastai/ui").UiMode} */ ("advanced"), theme: LIGHT_THEME };
  /** @type {MockHost} */
  const target = {
    host: /** @type {any} */ (null),
    calls: [],
    emit: (event) =>
      subscribers
        .filter((subscriber) => eventMatches(subscriber.filter, event))
        .forEach((subscriber) => subscriber.onEvent(event)),
    setDisplay: (mode, theme) => {
      display.mode = mode;
      display.theme = theme;
      displayListeners.forEach((listener) => listener());
    },
    routes: [],
    contexts: [],
    shownDiagnostics: [],
    contributions: {},
  };
  /** @type {HostServices} */
  const services = {
    executor: {
      execute: (commandId, args, options) => {
        target.calls.push({ commandId, args, options: options ?? {} });
        return client.execute(commandId, args, options);
      },
    },
    scopeOf: (commandId) => COMMAND_SCOPES[commandId] ?? "",
    subscribe: (filter, onEvent) => {
      const subscriber = { filter, onEvent };
      subscribers.push(subscriber);
      return () => {
        subscribers = subscribers.filter((other) => other !== subscriber);
      };
    },
    mode: () => display.mode,
    theme: () => display.theme,
    onDisplayChange: (listener) => {
      displayListeners.push(listener);
      return () => {
        displayListeners = displayListeners.filter((other) => other !== listener);
      };
    },
    navigate: (route) => target.routes.push(route),
    setContext: (_pluginId, values) => target.contexts.push(values),
    contributions: (kind) => target.contributions[kind] ?? [],
    showDiagnostics: (_pluginId, diagnostics) => target.shownDiagnostics.push(diagnostics),
  };
  target.host = createInProcessHost(pluginId, grantedScopes, services);
  return target;
}

/**
 * The host an executor case runs on: core, unless the case is about the acting plugin id, which only a plugin's
 * host sends (a caller's own actingPluginId option is ignored).
 * @param {ExecutorContractCase} contractCase
 * @returns {string}
 */
function pluginIdFor(contractCase) {
  return contractCase.options?.actingPluginId || CORE_PLUGIN_ID;
}

/**
 * Sets the fake daemon up to reply as the case says, as @rastai/client's own contract test does.
 * @param {ExecutorContractCase} contractCase
 */
function wireReply(contractCase) {
  const reply = contractCase.reply;
  if (reply.kind === "answer") {
    daemon.answerExecute = () => reply.response ?? {};
  } else if (reply.kind === "unreachable") {
    daemon.reachable = false;
  } else {
    daemon.refusal = SESSION_REFUSED_CODE;
  }
}

/**
 * The outcome a DaemonClient resolves to for the case's reply, as the mock host's canned outcome.
 * @param {ExecutorContractCase} contractCase
 * @returns {ExecuteOutcome}
 */
function cannedOutcomeFor(contractCase) {
  const reply = contractCase.reply;
  if (reply.kind === "unreachable" || reply.kind === "session_refused") {
    const code = reply.kind === "unreachable" ? UNREACHABLE_CODE : SIGNED_OUT_CODE;
    return { ok: false, result: null, jobId: null, diagnostics: [pluginDiagnostic(code, {})], journalId: null };
  }
  /** @type {any} */
  const response = reply.response ?? {};
  return {
    ok: response.ok ?? false,
    result: response.ok && response.resultJson ? JSON.parse(response.resultJson) : null,
    jobId: response.jobId || null,
    diagnostics: (response.diagnostics ?? []).map((/** @type {any} */ d) => pluginDiagnostic(d.code, {})),
    journalId: response.journalId || null,
  };
}

describe("the executor contract table", () => {
  test.each(EXECUTOR_CONTRACT_CASES)("on the in-process host: $name", async (contractCase) => {
    const client = await connect();
    const target = inProcessTarget(pluginIdFor(contractCase), ["read"], client);
    wireReply(contractCase);
    const outcome = await target.host.execute(contractCase.commandId, contractCase.args, contractCase.options);
    expect(summarizeOutcome(outcome)).toEqual(contractCase.outcome);
    if (contractCase.request === null) {
      expect(daemon.executeRequests).toEqual([]);
      return;
    }
    expect(daemon.executeRequests).toHaveLength(1);
    const received = daemon.executeRequests[0];
    expect({
      commandId: received.commandId,
      args: JSON.parse(received.argumentsJson),
      projectId: received.projectId,
      dryRun: received.dryRun,
      actingPluginId: received.actingPluginId,
    }).toEqual(contractCase.request);
  });

  // The mock has no daemon: it answers with the outcome a DaemonClient gives for the reply. What the table checks
  // here is the host's side: no throw, the outcome passed back as it came, the options and acting id it sends.
  test.each(EXECUTOR_CONTRACT_CASES)("on the mock host: $name", async (contractCase) => {
    const mock = createMockHost(pluginIdFor(contractCase), {
      [contractCase.commandId]: () => cannedOutcomeFor(contractCase),
    });
    const outcome = await mock.host.execute(contractCase.commandId, contractCase.args, contractCase.options);
    expect(summarizeOutcome(outcome)).toEqual(contractCase.outcome);
    expect(mock.calls).toHaveLength(1);
    if (contractCase.request !== null) {
      const call = mock.calls[0];
      expect({ commandId: call.commandId, args: call.args, ...call.options }).toEqual(contractCase.request);
    }
  });
});

describe("the host contract table", () => {
  test.each(HOST_CONTRACT_CASES)("on the in-process host: $name", async (contractCase) => {
    const target = inProcessTarget(HOST_CONTRACT_PLUGIN_ID, ["read"], await connect());
    expect(await contractCase.run(target)).toEqual(contractCase.expected);
  });

  test.each(HOST_CONTRACT_CASES)("on the mock host: $name", async (contractCase) => {
    const mock = createMockHost(HOST_CONTRACT_PLUGIN_ID, {
      "job.list": () => ({ ok: true, result: { jobs: [] }, jobId: null, diagnostics: [], journalId: null }),
    });
    expect(await contractCase.run(mock)).toEqual(contractCase.expected);
  });
});
