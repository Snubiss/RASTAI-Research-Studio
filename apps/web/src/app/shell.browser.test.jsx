// The shell's stores follow the daemon's events (layout frontend-and-plugins.md 8.1): settings.set's completion re-reads
// the settings and no other command does; plugin and node changes re-read their lists. The events arrive through the
// fake stream, as the daemon would send them.

import { describe, expect, test, vi } from "vitest";

import { answered, busEvent, testShell } from "../testing/fake-shell.js";
import { inAct } from "../testing/render.js";

/**
 * A command.finished event for one command (CommandFinished, contracts 7.3).
 * @param {string} commandId
 */
function commandFinished(commandId) {
  const value = { journalId: "journal_01", commandId, principal: "user:test", ok: true, code: "" };
  return busEvent("command.finished", "journal_01", { payload: { case: "commandFinished", value } });
}

/**
 * @param {{ calls: { commandId: string }[] }} client
 * @param {string} commandId
 */
function callsTo(client, commandId) {
  return client.calls.filter((/** @type {{ commandId: string }} */ call) => call.commandId === commandId).length;
}

/** Lets the coalesced refresh (a timer after the event) run. */
function settle() {
  return new Promise((resolve) => setTimeout(resolve, 20));
}

describe("the settings follow settings.set only", () => {
  test("another command finishing does not re-read the settings", async () => {
    const { client, stream } = await testShell({
      answers: { "settings.get": answered({ values: {}, sources: [] }) },
    });
    await inAct(() => stream.emit(commandFinished("system.hardware")));
    await settle();
    expect(callsTo(client, "settings.get")).toBe(0);
  });

  test("settings.set finishing re-reads the settings", async () => {
    const { client, stream } = await testShell({
      answers: { "settings.get": answered({ values: {}, sources: [] }) },
    });
    await inAct(() => stream.emit(commandFinished("settings.set")));
    await vi.waitFor(() => expect(callsTo(client, "settings.get")).toBe(1));
  });
});

describe("the lists follow their events", () => {
  test("plugin.state_changed re-reads the plugin list", async () => {
    const { shell, stream } = await testShell({
      answers: { "plugin.list": answered({ plugins: [{ plugin_id: "example" }] }) },
    });
    expect(shell.stores.plugins.get()).toEqual([]);
    const value = { pluginId: "example", fromState: 0, toState: 1 };
    await inAct(() =>
      stream.emit(busEvent("plugin.state_changed", "example", { payload: { case: "pluginStateChanged", value } })),
    );
    await vi.waitFor(() => expect(shell.stores.plugins.get()).toEqual([{ plugin_id: "example" }]));
  });

  test("node.state_changed re-reads the node list", async () => {
    const { shell, stream } = await testShell({
      answers: { "node.list": answered({ nodes: [{ node_id: "node_local", name: "local" }] }) },
    });
    const value = { nodeId: "node_local", fromState: 0, toState: 1 };
    await inAct(() =>
      stream.emit(busEvent("node.state_changed", "node_local", { payload: { case: "nodeStateChanged", value } })),
    );
    await vi.waitFor(() => expect(shell.stores.nodes.get()).toEqual([{ node_id: "node_local", name: "local" }]));
  });
});
