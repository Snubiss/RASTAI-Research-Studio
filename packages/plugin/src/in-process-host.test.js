// The in-process host's own rules (layout frontend-and-plugins.md 7.2; contracts 6.3): a plugin's call outside its
// commands:[scopes] permission is refused before it leaves the window, the acting plugin id is always the host's,
// and the shell learns which plugin set a context or showed a diagnostic. The shared tables run against it in
// testing/host-contract.test.js.

import { describe, expect, test } from "vitest";

import { CORE_PLUGIN_ID, createInProcessHost, mayRun } from "./in-process-host.js";

/** @typedef {import("./in-process-host.js").HostServices} HostServices */
/** @typedef {import("./host-api.js").ExecuteOptions} ExecuteOptions */

// What the command index lists: command id -> scope (contracts 6.3 and 6.4).
/** @type {Object<string, string>} */
const SCOPES = {
  "job.list": "read",
  "model.download": "inference.manage",
  "settings.set": "system.admin",
  "psychology.sae.load": "psychology",
  "acme_lens.render": "plugin:acme_lens",
};

/** An app around the host that records what reaches it. */
function fakeApp() {
  /** @type {{ commandId: string, args: object, options: ExecuteOptions | undefined }[]} */
  const executed = [];
  /** @type {string[]} */
  const shellCalls = [];
  /** @type {HostServices} */
  const services = {
    executor: {
      execute: async (commandId, args, options) => {
        executed.push({ commandId, args, options });
        return { ok: true, result: null, jobId: null, diagnostics: [], journalId: null };
      },
    },
    scopeOf: (commandId) => SCOPES[commandId] ?? "",
    subscribe: () => () => {},
    mode: () => "advanced",
    theme: () => {
      throw new Error("not used here");
    },
    onDisplayChange: () => () => {},
    navigate: () => {},
    setContext: (pluginId, values) => shellCalls.push(`context ${pluginId} ${JSON.stringify(values)}`),
    contributions: () => [],
    showDiagnostics: (pluginId, diagnostics) => shellCalls.push(`diagnostics ${pluginId} ${diagnostics.length}`),
  };
  return { services, executed, shellCalls };
}

describe("execute", () => {
  test("refuses a command outside the plugin's scopes with RRS-C1005, before it reaches the daemon", async () => {
    const app = fakeApp();
    const host = createInProcessHost("psychology", ["psychology", "read"], app.services);
    const outcome = await host.execute("settings.set", { key: "ui.theme", value: "dark" });
    expect(outcome.ok).toBe(false);
    expect(outcome.result).toBeNull();
    expect(outcome.diagnostics).toHaveLength(1);
    expect(outcome.diagnostics[0].code).toBe("RRS-C1005");
    expect(outcome.diagnostics[0].severity).toBe(1);
    expect(outcome.diagnostics[0].message).toBe(
      "settings.set needs the scope system.admin, which plugin:psychology wasn't granted.",
    );
    expect(app.executed).toEqual([]);
  });

  test("passes a command inside the granted scopes, with the plugin as the acting plugin", async () => {
    const app = fakeApp();
    const host = createInProcessHost("psychology", ["psychology", "inference.manage"], app.services);
    await host.execute("model.download", { model_ref: "gemma-3-1b" }, { projectId: "prj_01" });
    expect(app.executed).toEqual([
      {
        commandId: "model.download",
        args: { model_ref: "gemma-3-1b" },
        options: { projectId: "prj_01", dryRun: false, actingPluginId: "psychology" },
      },
    ]);
  });

  test("core runs as the owner: no scope check and no acting plugin id, whatever the caller passed", async () => {
    const app = fakeApp();
    const host = createInProcessHost(CORE_PLUGIN_ID, [], app.services);
    const outcome = await host.execute("settings.set", { key: "ui.theme", value: "dark" }, { actingPluginId: "evil" });
    expect(outcome.ok).toBe(true);
    expect(app.executed[0].options).toEqual({ projectId: "", dryRun: false, actingPluginId: "" });
  });
});

describe("mayRun", () => {
  test.each([
    ["a granted scope", "psychology", ["read"], "read", true],
    ["a scope not granted", "psychology", ["read"], "secrets", false],
    ["the plugin's own scope", "acme_lens", [], "plugin:acme_lens", true],
    ["another plugin's own scope", "acme_lens", [], "plugin:psychology", false],
    ["a command the index doesn't list (the daemon decides)", "acme_lens", [], "", true],
    ["anything, for core", CORE_PLUGIN_ID, [], "system.admin", true],
  ])("%s", (_name, pluginId, grantedScopes, scope, expected) => {
    expect(mayRun(pluginId, grantedScopes, scope)).toBe(expected);
  });
});

describe("the shell's side", () => {
  test("setContext and showDiagnostics name the plugin they came from", () => {
    const app = fakeApp();
    const host = createInProcessHost("eval", ["eval"], app.services);
    host.setContext({ "eval.suite": "doc_01" });
    host.showDiagnostics([]);
    expect(app.shellCalls).toEqual(['context eval {"eval.suite":"doc_01"}', "diagnostics eval 0"]);
  });
});
