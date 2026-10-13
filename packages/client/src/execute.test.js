// execute and listCommands against the fake daemon (layout frontend-and-plugins.md 3.5), including the shared
// executor contract table run against a DaemonClient.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { connectToDaemon } from "./connection.js";
import { listCommands } from "./execute.js";
import { settingsGet } from "./generated/commands.js";
import { EXECUTOR_CONTRACT_CASES, summarizeOutcome } from "./testing/executor-contract.js";
import { createFakeDaemon, refusalError } from "./testing/fake-daemon.js";

/** @typedef {import("./testing/fake-daemon.js").FakeDaemon} FakeDaemon */
/** @typedef {import("./testing/executor-contract.js").ExecutorContractCase} ExecutorContractCase */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */
/** @typedef {import("./command-executor.js").CommandExecutor} CommandExecutor */

const SESSION_REFUSED_CODE = "RRS-C1002";

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
 * Sets the fake daemon up to reply as the case says. Connecting comes first, so an unreachable daemon is one that
 * went away after the client connected.
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

describe("the executor contract table, against a DaemonClient on the fake router", () => {
  test.each(EXECUTOR_CONTRACT_CASES)("$name", async (contractCase) => {
    /** @type {CommandExecutor} */
    const executor = await connect();
    wireReply(contractCase);
    const outcome = await executor.execute(contractCase.commandId, contractCase.args, contractCase.options);
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
});

describe("execute", () => {
  test("gives each call a fresh idempotency key", async () => {
    const client = await connect();
    await client.execute("system.status", {});
    await client.execute("system.status", {});
    const keys = daemon.executeRequests.map((request) => request.idempotencyKey);
    expect(keys[0]).not.toBe("");
    expect(keys[0]).not.toBe(keys[1]);
  });

  test("sends a call that got no answer once more with the same key", async () => {
    const client = await connect();
    let calls = 0;
    daemon.answerExecute = () => {
      calls += 1;
      if (calls === 1) {
        // A connection that broke before the answer came: no Diagnostic of the daemon's.
        throw new Error("connection reset");
      }
      return { ok: true, resultJson: '{"done":true}' };
    };
    const outcome = await client.execute("system.status", {});
    expect(outcome.ok).toBe(true);
    expect(outcome.result).toEqual({ done: true });
    expect(daemon.executeRequests).toHaveLength(2);
    expect(daemon.executeRequests[1].idempotencyKey).toBe(daemon.executeRequests[0].idempotencyKey);
  });

  test("returns a refusal the daemon made as it came, without retrying", async () => {
    const client = await connect();
    daemon.answerExecute = () => {
      throw refusalError("RRS-C1004");
    };
    const outcome = await client.execute("system.status", {});
    expect(summarizeOutcome(outcome).diagnosticCodes).toEqual(["RRS-C1004"]);
    expect(daemon.executeRequests).toHaveLength(1);
  });

  test("the generated typed wrappers run against a DaemonClient", async () => {
    const client = await connect();
    daemon.answerExecute = () => ({ ok: true, resultJson: '{"key":"ui.theme","value":"dark","source":"user"}' });
    const outcome = await settingsGet(client, { key: "ui.theme" });
    expect(outcome.result).toEqual({ key: "ui.theme", value: "dark", source: "user" });
    expect(daemon.executeRequests[0].commandId).toBe("settings.get");
    expect(JSON.parse(daemon.executeRequests[0].argumentsJson)).toEqual({ key: "ui.theme" });
  });
});

describe("listCommands", () => {
  test("returns the daemon's command descriptors", async () => {
    const client = await connect();
    daemon.commands = [{ commandId: "system.status", title: "System status" }];
    const commands = await listCommands(client);
    expect(commands.map((command) => [command.commandId, command.title])).toEqual([["system.status", "System status"]]);
  });

  test("rejects with the RRS-U Diagnostic as the cause when the daemon is gone", async () => {
    const client = await connect();
    daemon.reachable = false;
    const failure = await listCommands(client).catch((error) => error);
    expect(failure).toBeInstanceOf(Error);
    expect(failure.cause.code).toBe("RRS-U0001");
  });
});
