// The loader is the one place UIs are registered (layout frontend-and-plugins.md 10.2). Its first run registers the
// core sections; a second run is refused, and the first registration is still the one the window holds.

import { registeredPluginUi } from "@rastai/plugin";
import { describe, expect, test } from "vitest";

import { registerShellUis } from "./load-plugins.js";
import { sectionById } from "./plugin-registry.js";

describe("registerShellUis", () => {
  test("registers the core sections under the id core, with their panels", async () => {
    const result = await registerShellUis([{ plugin_id: "psychology", state: "disabled" }]);
    expect(result.loadedIds).toEqual(["core"]);
    expect(result.diagnostics).toEqual([]);
    expect(sectionById("home", ["core"])?.id).toBe("home");
    expect(registeredPluginUi("core")?.panels.map((panel) => panel.id)).toContain("core.home");
  });

  test("a second registration of the core is refused, and the first one stays", async () => {
    const first = registeredPluginUi("core");
    const result = await registerShellUis([]);
    expect(result.loadedIds).toEqual([]);
    expect(result.diagnostics.map((diagnostic) => diagnostic.code)).toEqual(["RRS-P4105"]);
    expect(registeredPluginUi("core")).toBe(first);
  });
});

describe("sectionById", () => {
  test("a section no registered UI offers is null", () => {
    expect(sectionById("psychology", ["core"])).toBeNull();
  });
});
