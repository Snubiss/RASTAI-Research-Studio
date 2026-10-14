// The shell's stores follow the daemon's events (layout frontend-and-plugins.md 8.1): settings.set's completion re-reads
// the settings and no other command does; plugin and node changes re-read their lists. The events arrive through the
// fake stream, as the daemon would send them.

import { describe, expect, test, vi } from "vitest";

import { answered, busEvent, refused, testShell } from "../testing/fake-shell.js";
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

describe("a refused setting puts back only its own key", () => {
  /**
   * A settings.set the test answers by hand: each write waits in `pending` until the test resolves it.
   * @returns {{ pending: { args: any, resolve: (outcome: any) => void }[], answer: (args: any) => Promise<any> }}
   */
  function heldWrites() {
    /** @type {{ args: any, resolve: (outcome: any) => void }[]} */
    const pending = [];
    return {
      pending,
      answer: (args) => new Promise((resolve) => pending.push({ args, resolve })),
    };
  }

  /** @param {any[]} pending @param {string} key */
  function writeFor(pending, key) {
    return pending.find((write) => write.args.key === key);
  }

  /** @param {any[]} pending @param {any} value */
  function writeWithValue(pending, value) {
    return pending.find((write) => write.args.value === value);
  }

  test("a refusal of one key keeps a later change to another key", async () => {
    const daemon = { ui: { density: "comfortable", reduced_motion: false } };
    const writes = heldWrites();
    const { shell } = await testShell({
      answers: {
        "settings.get": () => answered({ values: structuredClone(daemon), sources: [] }),
        "settings.set": writes.answer,
      },
    });
    // The window reads the settings at boot, before any change; the writes start from those stored values.
    shell.applySettings({ values: structuredClone(daemon), sources: [] });
    const first = shell.setSetting("ui.density", "compact");
    const second = shell.setSetting("ui.reduced_motion", true);
    daemon.ui.reduced_motion = true;
    writeFor(writes.pending, "ui.reduced_motion").resolve(answered({}));
    await second;
    writeFor(writes.pending, "ui.density").resolve(refused("RRS-C1005", "settings.set"));
    await first;
    const values = shell.stores.settings.get().values;
    expect(values.ui.reduced_motion).toBe(true);
    expect(values.ui.density).toBe("comfortable");
  });

  test("a refusal does not undo a later change to the same key", async () => {
    const daemon = { ui: { density: "comfortable", reduced_motion: false } };
    const writes = heldWrites();
    const { shell } = await testShell({
      answers: {
        "settings.get": () => answered({ values: structuredClone(daemon), sources: [] }),
        "settings.set": writes.answer,
      },
    });
    // The window reads the settings at boot, before any change; the writes start from those stored values.
    shell.applySettings({ values: structuredClone(daemon), sources: [] });
    const first = shell.setSetting("ui.density", "compact");
    const second = shell.setSetting("ui.density", "dense");
    daemon.ui.density = "dense";
    writeWithValue(writes.pending, "dense").resolve(answered({}));
    await second;
    writeWithValue(writes.pending, "compact").resolve(refused("RRS-C1005", "settings.set"));
    await first;
    expect(shell.stores.settings.get().values.ui.density).toBe("dense");
  });

  test("two refused writes to one key show the value the daemon kept", async () => {
    const daemon = { ui: { density: "comfortable", reduced_motion: false } };
    const writes = heldWrites();
    const { shell } = await testShell({
      answers: {
        "settings.get": () => answered({ values: structuredClone(daemon), sources: [] }),
        "settings.set": writes.answer,
      },
    });
    // The window reads the settings at boot, before any change; the writes start from those stored values.
    shell.applySettings({ values: structuredClone(daemon), sources: [] });
    const first = shell.setSetting("ui.density", "compact");
    const second = shell.setSetting("ui.density", "dense");
    // Both are refused, so the second must not restore the first one's unconfirmed "compact".
    writeWithValue(writes.pending, "compact").resolve(refused("RRS-C1005", "settings.set"));
    await first;
    writeWithValue(writes.pending, "dense").resolve(refused("RRS-C1005", "settings.set"));
    await second;
    expect(shell.stores.settings.get().values.ui.density).toBe("comfortable");
  });
});
