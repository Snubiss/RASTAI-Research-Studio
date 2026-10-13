// registerPluginUi (layout frontend-and-plugins.md 7.2 and 10.2): a well-formed UI is registered under its plugin
// id; a malformed one throws, which the app's loader reports as RRS-P4105.

import { describe, expect, test } from "vitest";

import { pluginUiProblem, registerPluginUi, registeredPluginUi } from "./ui-registry.js";

/** @typedef {import("./ui-registry.js").PluginUi} PluginUi */

const render = () => null;

/**
 * A small first-party UI, as a plugin's index.js exports it.
 * @returns {PluginUi}
 */
function sampleUi() {
  return {
    pluginId: "psychology",
    sections: [{ id: "psychology", render }],
    panels: [{ id: "psychology.live", render }],
    lensViews: [{ id: "psychology.logit_lens", render }],
    metricViews: [],
    settingsPages: [{ id: "psychology.settings", render }],
  };
}

describe("registerPluginUi", () => {
  test("registers a UI under its plugin id, and a second registration for that id is refused", () => {
    const first = sampleUi();
    registerPluginUi(first);
    expect(registeredPluginUi("psychology")).toBe(first);
    expect(() => registerPluginUi(sampleUi())).toThrow(/already registered/);
    expect(registeredPluginUi("psychology")).toBe(first);
    expect(registeredPluginUi("eval")).toBeNull();
  });

  test("core sections keep their own names", () => {
    const core = {
      ...sampleUi(),
      pluginId: "core",
      sections: [{ id: "home", render }],
      panels: [{ id: "core.lineage", render }],
    };
    core.lensViews = [];
    core.settingsPages = [];
    expect(pluginUiProblem(core)).toBeNull();
  });

  test("throws on a malformed UI and registers nothing", () => {
    const broken = { ...sampleUi(), pluginId: "broken", panels: [{ id: "psychology.live", render }] };
    expect(() => registerPluginUi(broken)).toThrow(/must start with broken\./);
    expect(registeredPluginUi("broken")).toBeNull();
  });
});

describe("pluginUiProblem", () => {
  test.each([
    ["no plugin id", { pluginId: "" }, "pluginId must be a non-empty string"],
    ["a list that isn't one", { metricViews: null }, "metricViews must be a list"],
    ["an entry without render", { panels: [{ id: "psychology.live" }] }, "needs an id and a render function"],
    [
      "an id twice",
      {
        panels: [
          { id: "psychology.live", render },
          { id: "psychology.live", render },
        ],
      },
      "twice",
    ],
    ["an id outside the namespace", { lensViews: [{ id: "eval.pass_at_k", render }] }, "must start with psychology."],
    ["a section with another plugin's id", { sections: [{ id: "eval", render }] }, "isn't the plugin's own id"],
  ])("finds %s", (_name, change, expected) => {
    const ui = /** @type {PluginUi} */ ({ ...sampleUi(), ...change });
    expect(pluginUiProblem(ui)).toContain(expected);
  });
});
