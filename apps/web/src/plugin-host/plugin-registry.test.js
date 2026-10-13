// What the enabled plugins contribute (contracts 5.5): each enabled plugin's entries of one kind, with its id. A disabled
// plugin contributes nothing.

import { describe, expect, test } from "vitest";

import { contributionsOf } from "./plugin-registry.js";

const PLUGINS = [
  {
    plugin_id: "psychology",
    state: "enabled",
    manifest: { contributes: { lenses: [{ id: "psychology.logit_lens", title: "Logit lens" }] } },
  },
  {
    plugin_id: "eval",
    state: "disabled",
    manifest: { contributes: { lenses: [{ id: "eval.tile", title: "Tile" }] } },
  },
  { plugin_id: "acme", state: "enabled", manifest: {} },
];

describe("contributionsOf", () => {
  test("lists the entries of one kind from the enabled plugins only", () => {
    expect(contributionsOf(PLUGINS, "lenses")).toEqual([
      {
        pluginId: "psychology",
        id: "psychology.logit_lens",
        title: "Logit lens",
        data: { id: "psychology.logit_lens", title: "Logit lens" },
      },
    ]);
  });

  test("a kind no plugin has is an empty list", () => {
    expect(contributionsOf(PLUGINS, "metrics")).toEqual([]);
  });
});
