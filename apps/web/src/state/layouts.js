// The layout tree of each section in this window (layout frontend-and-plugins.md 2.6). Kept in the layouts store, so a
// change re-renders the section; nothing is saved (workspace.layout.save is M6).

import { defaultLayout } from "../workspace/layout-tree.js";

/** @typedef {import("../workspace/layout-tree.js").LayoutNode} LayoutNode */
/** @typedef {import("@rastai/plugin").Store} Store */

/**
 * The section's tree: the one the window holds, or the default with its one panel.
 * @param {any} layouts   the layouts store's value: { [sectionId]: LayoutNode }
 * @param {string} sectionId
 * @param {string} panelId
 * @returns {LayoutNode}
 */
export function layoutOf(layouts, sectionId, panelId) {
  return layouts[sectionId] ?? defaultLayout(panelId);
}

/**
 * Stores a section's tree.
 * @param {Store} layoutsStore
 * @param {string} sectionId
 * @param {LayoutNode} node
 */
export function setLayoutOf(layoutsStore, sectionId, node) {
  layoutsStore.set({ ...layoutsStore.get(), [sectionId]: node });
}
