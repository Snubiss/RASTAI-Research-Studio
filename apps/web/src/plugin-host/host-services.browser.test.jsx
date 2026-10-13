// The window's real HostServices, run against the shared host table (layout frontend-and-plugins.md 7.5): the same
// cases @rastai/plugin runs against its mock and its in-process host. The services here are the app's own (its client,
// hub, display and stores), with the daemon and the event stream faked, so a case that passes shows the app keeps the
// promises: subscribe filters, locked tokens excluded, scopes checked before a call leaves the window.

import { HOST_CONTRACT_CASES, HOST_CONTRACT_PLUGIN_ID } from "@rastai/plugin/testing/host-contract";
import { builtInTheme } from "@rastai/ui";
import { createInProcessHost } from "@rastai/plugin";
import { describe, expect, test } from "vitest";

import { answered, testShell } from "../testing/fake-shell.js";
import { hostServicesOf } from "./in-process-host.js";

const COMMANDS = [
  { commandId: "job.list", scope: "read" },
  { commandId: "settings.set", scope: "system.admin" },
];
const DARK_COMPACT = builtInTheme("dark", { density: "compact", reducedMotion: true, colorblind: false });

/**
 * A plugin's host over the window's services, with the table's levers: the stream, the display, the routes, the
 * contexts, the diagnostics and the contributions. Each lever reads or writes the app's own stores.
 * @param {string[]} grantedScopes
 */
async function windowTarget(grantedScopes) {
  const { shell, client, stream } = await testShell({
    commands: COMMANDS,
    answers: { "job.list": answered({ jobs: [] }) },
  });
  const host = createInProcessHost(HOST_CONTRACT_PLUGIN_ID, grantedScopes, hostServicesOf(shell));
  return {
    host,
    /** @param {any} event */
    emit: (event) => stream.emit(event),
    /** @param {any} mode @param {any} theme */
    setDisplay: (mode, theme) => shell.stores.display.set({ mode, theme }),
    get calls() {
      return client.calls;
    },
    get routes() {
      return [window.location.hash];
    },
    get contexts() {
      const values = shell.stores.context.get()[HOST_CONTRACT_PLUGIN_ID];
      return values === undefined ? [] : [values];
    },
    get shownDiagnostics() {
      // Each toast is keyed by its plugin and code; the table reads the code of each diagnostic shown.
      return [shell.stores.notices.get().map((/** @type {any} */ toast) => ({ code: toast.id.split(":")[1] }))];
    },
    contributions: contributionsLever(shell),
  };
}

/**
 * `contributions.<kind> = [...]` sets what the enabled plugin contributes of that kind, through its manifest, as a
 * plugin.list answer would.
 * @param {any} shell
 */
function contributionsLever(shell) {
  return new Proxy(
    {},
    {
      set(_target, kind, entries) {
        const contributes = {
          [String(kind)]: entries.map((/** @type {any} */ entry) => ({ id: entry.id, title: entry.title })),
        };
        shell.stores.plugins.set([{ plugin_id: HOST_CONTRACT_PLUGIN_ID, state: "enabled", manifest: { contributes } }]);
        return true;
      },
    },
  );
}

describe("the host contract on the window's services", () => {
  test.each(HOST_CONTRACT_CASES)("$name", async (contractCase) => {
    const target = await windowTarget(["read"]);
    expect(await contractCase.run(target)).toEqual(contractCase.expected);
  });

  test("the theme a host gets carries no locked security color, even if the display's theme names one", async () => {
    const target = await windowTarget(["read"]);
    target.setDisplay("basic", {
      ...DARK_COMPACT,
      colors: { ...DARK_COMPACT.colors, "severity-error": "hostile-value" },
    });
    expect(target.host.theme().colors).not.toHaveProperty("severity-error");
    expect(target.host.theme().colors.surface).toBe(DARK_COMPACT.colors.surface);
  });

  test("a call outside the plugin's scopes is refused before it reaches the daemon", async () => {
    const target = await windowTarget(["read"]);
    const outcome = await target.host.execute("settings.set", { key: "ui.theme", value: "dark" });
    expect(outcome.ok).toBe(false);
    expect(outcome.diagnostics[0].code).toBe("RRS-C1005");
    expect(target.calls).toEqual([]);
  });
});
