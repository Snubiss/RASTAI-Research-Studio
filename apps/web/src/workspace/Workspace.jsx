// Draws one layout tree: a split is its children side by side or stacked, a tab group shows one child at a time, and a
// panel is a PanelFrame (layout frontend-and-plugins.md 2.6). The tree is data; this component only draws it.

import { useState } from "react";

import { PanelFrame } from "./PanelFrame.jsx";

/** @typedef {import("./layout-tree.js").LayoutNode} LayoutNode */

/**
 * @param {object} props
 * @param {LayoutNode} props.node
 * @param {string} props.sectionId
 */
export function Workspace({ node, sectionId }) {
  if (node.kind === "panel") {
    return <PanelFrame sectionId={sectionId} panelId={node.panelId} modeOverride={node.modeOverride} />;
  }
  if (node.kind === "split") {
    return (
      <div className={`rr-workspace-split rr-workspace-split--${node.direction}`}>
        {node.children.map((child, index) => (
          <Workspace key={index} node={child} sectionId={sectionId} />
        ))}
      </div>
    );
  }
  return <TabGroup node={node} sectionId={sectionId} />;
}

/**
 * A tab group: one tab per child, the active child below. Which tab is open is this window's own view state, so it
 * is kept here and not in the layout tree.
 * @param {object} props
 * @param {import("./layout-tree.js").TabsNode} props.node
 * @param {string} props.sectionId
 */
function TabGroup({ node, sectionId }) {
  const [activeIndex, setActiveIndex] = useState(node.activeIndex);
  const active = node.children[activeIndex] ?? node.children[0];
  /** @param {number} index */
  function activate(index) {
    setActiveIndex(index);
  }
  return (
    <div className="rr-workspace-tabs">
      <div role="tablist" className="rr-workspace-tabs__list">
        {node.children.map((child, index) => (
          <button
            key={index}
            type="button"
            role="tab"
            aria-selected={index === activeIndex}
            className="rr-workspace-tabs__tab"
            onClick={() => activate(index)}
          >
            {child.kind === "panel" ? child.panelId : "Group"}
          </button>
        ))}
      </div>
      <Workspace node={active} sectionId={sectionId} />
    </div>
  );
}
