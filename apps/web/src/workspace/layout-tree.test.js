// The layout tree's pure functions (layout frontend-and-plugins.md 2.6; design 3.3). Every call returns a new tree, and
// the one it was given is left as it was.

import { describe, expect, test } from "vitest";

import { defaultLayout, findPanel, movePanel, setModeOverride } from "./layout-tree.js";

/** @typedef {import("./layout-tree.js").LayoutNode} LayoutNode */
/** @typedef {import("./layout-tree.js").PanelNode} PanelNode */
/** @typedef {import("./layout-tree.js").SplitNode} SplitNode */

/**
 * @param {string} panelId
 * @returns {PanelNode}
 */
function panel(panelId) {
  return { kind: "panel", panelId, modeOverride: "" };
}

/**
 * @param {"row" | "column"} direction
 * @param {LayoutNode[]} children
 * @returns {SplitNode}
 */
function split(direction, children) {
  return { kind: "split", direction, children };
}

/** A two-panel split: the chat beside the live panel, the way Inference's default layout puts psychology.live. */
function twoPanels() {
  return split("row", [panel("core.inference"), panel("psychology.live")]);
}

describe("defaultLayout", () => {
  test("a section starts with its one panel, following the app's mode", () => {
    expect(defaultLayout("core.home")).toEqual({ kind: "panel", panelId: "core.home", modeOverride: "" });
  });
});

describe("findPanel", () => {
  test("finds a panel anywhere in the tree, or says there is none", () => {
    const tree = twoPanels();
    expect(findPanel(tree, "psychology.live")?.panelId).toBe("psychology.live");
    expect(findPanel(tree, "core.lineage")).toBeNull();
  });
});

describe("setModeOverride", () => {
  test("changes one panel's override and no other panel's", () => {
    const tree = twoPanels();
    const changed = setModeOverride(tree, "core.inference", "basic");
    expect(findPanel(changed, "core.inference")?.modeOverride).toBe("basic");
    expect(findPanel(changed, "psychology.live")?.modeOverride).toBe("");
    expect(findPanel(tree, "core.inference")?.modeOverride).toBe("");
  });

  test("an empty override follows the app again", () => {
    const tree = setModeOverride(twoPanels(), "core.inference", "advanced");
    expect(findPanel(setModeOverride(tree, "core.inference", ""), "core.inference")?.modeOverride).toBe("");
  });
});

describe("movePanel", () => {
  test("moves a panel to the right of the rest, as a row split", () => {
    expect(movePanel(twoPanels(), "psychology.live", "right")).toEqual(
      split("row", [panel("core.inference"), panel("psychology.live")]),
    );
  });

  test("moves a panel to the bottom, as a column split", () => {
    expect(movePanel(twoPanels(), "core.inference", "bottom")).toEqual(
      split("column", [panel("psychology.live"), panel("core.inference")]),
    );
  });

  test("moves a panel to a new tab group, with the moved panel active", () => {
    expect(movePanel(twoPanels(), "core.inference", "new_tab")).toEqual({
      kind: "tabs",
      children: [panel("psychology.live"), panel("core.inference")],
      activeIndex: 1,
    });
  });

  test("the only panel of a tree stays where it is", () => {
    const only = defaultLayout("core.home");
    expect(movePanel(only, "core.home", "right")).toEqual(only);
  });

  test("a split left with one child collapses into that child", () => {
    const inner = split("column", [panel("core.a"), panel("core.b")]);
    const tree = split("row", [inner, panel("core.c")]);
    expect(movePanel(tree, "core.b", "new_tab")).toEqual({
      kind: "tabs",
      children: [split("row", [panel("core.a"), panel("core.c")]), panel("core.b")],
      activeIndex: 1,
    });
  });

  test("a panel id the tree doesn't hold changes nothing", () => {
    const tree = twoPanels();
    expect(movePanel(tree, "core.lineage", "right")).toBe(tree);
  });
});
