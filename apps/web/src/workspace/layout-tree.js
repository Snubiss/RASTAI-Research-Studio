// The layout tree of one section (layout frontend-and-plugins.md 2.6; design 3.3). Pure functions: each returns a new
// tree and never changes the one it is given, so the tree can live in a store. A tree is a panel, a split (its
// children side by side or stacked) or a tab group. Saved layouts are files (workspace.layout.save, M6); this module
// only holds the layout a window has now.

/**
 * A panel leaf. `modeOverride` is "" to follow the app's mode (the layout's mode_override).
 * @typedef {object} PanelNode
 * @property {"panel"} kind
 * @property {string} panelId
 * @property {"" | "basic" | "advanced"} modeOverride
 */

/**
 * Children side by side ("row") or one above the other ("column").
 * @typedef {object} SplitNode
 * @property {"split"} kind
 * @property {"row" | "column"} direction
 * @property {LayoutNode[]} children
 */

/**
 * Children as tabs; the active one is shown.
 * @typedef {object} TabsNode
 * @property {"tabs"} kind
 * @property {LayoutNode[]} children
 * @property {number} activeIndex
 */

/** @typedef {PanelNode | SplitNode | TabsNode} LayoutNode */

/**
 * The layout a section starts with: its one panel.
 * @param {string} panelId
 * @returns {LayoutNode}
 */
export function defaultLayout(panelId) {
  return { kind: "panel", panelId, modeOverride: "" };
}

/**
 * The panel with this id, or null when the tree has none.
 * @param {LayoutNode} node
 * @param {string} panelId
 * @returns {PanelNode | null}
 */
export function findPanel(node, panelId) {
  if (node.kind === "panel") {
    return node.panelId === panelId ? node : null;
  }
  for (const child of node.children) {
    const found = findPanel(child, panelId);
    if (found !== null) {
      return found;
    }
  }
  return null;
}

/**
 * The tree with one panel's mode override changed. Every other panel keeps its own.
 * @param {LayoutNode} node
 * @param {string} panelId
 * @param {"" | "basic" | "advanced"} modeOverride
 * @returns {LayoutNode}
 */
export function setModeOverride(node, panelId, modeOverride) {
  if (node.kind === "panel") {
    return node.panelId === panelId ? { ...node, modeOverride } : node;
  }
  return { ...node, children: node.children.map((child) => setModeOverride(child, panelId, modeOverride)) };
}

/**
 * The tree with one panel moved to the right, the bottom or a new tab of the rest. Moving the only panel of a tree
 * leaves it where it is, since there is nothing to dock beside.
 * @param {LayoutNode} node
 * @param {string} panelId
 * @param {"right" | "bottom" | "new_tab"} target
 * @returns {LayoutNode}
 */
export function movePanel(node, panelId, target) {
  const panel = findPanel(node, panelId);
  if (panel === null) {
    return node;
  }
  const rest = removePanel(node, panelId);
  if (rest === null) {
    return node;
  }
  if (target === "new_tab") {
    return { kind: "tabs", children: [rest, panel], activeIndex: 1 };
  }
  const direction = target === "right" ? "row" : "column";
  return { kind: "split", direction, children: [rest, panel] };
}

/**
 * The tree without one panel. A split or tab group left with one child is replaced by that child.
 * @param {LayoutNode} node
 * @param {string} panelId
 * @returns {LayoutNode | null}   null when the panel was the whole tree
 */
function removePanel(node, panelId) {
  if (node.kind === "panel") {
    return node.panelId === panelId ? null : node;
  }
  const children = node.children.map((child) => removePanel(child, panelId)).filter((child) => child !== null);
  if (children.length === 0) {
    return null;
  }
  if (children.length === 1) {
    return children[0];
  }
  if (node.kind === "tabs") {
    return { ...node, children, activeIndex: Math.min(node.activeIndex, children.length - 1) };
  }
  return { ...node, children };
}
