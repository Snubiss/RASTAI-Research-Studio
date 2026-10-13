// The package's public surface (layout frontend-and-plugins.md 7.2 and 7.4): index.js exports the registered names
// (contracts 15.3) and every core command's typed wrapper, nothing from the security UI, and no file of the package
// names the security subpath of @rastai/ui at all.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync, readdirSync } from "node:fs";

import * as commands from "@rastai/client/commands";
import { describe, expect, test } from "vitest";

import * as plugin from "./index.js";
import packageFile from "../package.json" with { type: "json" };

const SOURCE_FOLDER = new URL("./", import.meta.url);
// The runtime exports contracts 15.3 registers for this package (its typedefs aren't runtime values).
const REGISTERED_EXPORTS = [
  "CORE_PLUGIN_ID",
  "CommandForm",
  "JobProgress",
  "PluginHostProvider",
  "clearDraft",
  "createInProcessHost",
  "createLiveBuffer",
  "createMockHost",
  "createStore",
  "invalidateQueries",
  "itemsSince",
  "loadDraft",
  "pluginDiagnostic",
  "pushItem",
  "registerPluginUi",
  "registeredPluginUi",
  "saveDraft",
  "useCommand",
  "useLiveBuffer",
  "usePluginHost",
  "useQuery",
  "useStore",
];
const SECURITY_COMPONENTS = ["EgressBadge", "PermissionList", "SignedOutScreen", "TrustPrompt"];
// Written as joined parts, so this file doesn't contain the path it looks for.
const SECURITY_SUBPATH = "@rastai/ui/" + "security";

/**
 * Every hand-written file under src/, as a path relative to src/.
 * @param {string} folder   relative to src/, "" for src/ itself
 * @returns {string[]}
 */
function sourceFiles(folder) {
  /** @type {string[]} */
  const files = [];
  for (const entry of readdirSync(new URL(folder, SOURCE_FOLDER), { withFileTypes: true })) {
    const path = `${folder}${entry.name}`;
    if (entry.isDirectory()) {
      files.push(...sourceFiles(`${path}/`));
    } else {
      files.push(path);
    }
  }
  return files;
}

describe("exports", () => {
  test("index.js is the only entry point, besides the shared host table the app's tests run against", () => {
    // The table is test support: apps/web's host-services test runs it against the window's real services (M1-53).
    expect(packageFile.exports).toEqual({
      ".": "./src/index.js",
      "./testing/host-contract": "./src/testing/host-contract.js",
    });
  });

  test("index.js exports the registered names and the core command wrappers, nothing else", () => {
    const commandWrappers = Object.keys(commands);
    expect(commandWrappers.length).toBeGreaterThan(0);
    expect(Object.keys(plugin).sort()).toEqual([...REGISTERED_EXPORTS, ...commandWrappers].sort());
  });

  test("the wrappers are @rastai/client's own, so they take a PluginHost like any CommandExecutor", () => {
    const exported = /** @type {Object<string, unknown>} */ (plugin);
    for (const [name, wrapper] of Object.entries(commands)) {
      expect(exported[name], name).toBe(wrapper);
    }
  });
});

describe("the security UI stays in the shell", () => {
  test("no security component is exported", () => {
    for (const name of SECURITY_COMPONENTS) {
      expect(Object.keys(plugin)).not.toContain(name);
    }
  });

  test("no source file names the security subpath", () => {
    for (const file of sourceFiles("")) {
      const text = readFileSync(new URL(file, SOURCE_FOLDER), "utf8");
      expect(text.includes(SECURITY_SUBPATH), file).toBe(false);
    }
  });
});
