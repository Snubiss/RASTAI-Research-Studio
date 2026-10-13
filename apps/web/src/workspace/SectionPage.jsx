// A section's page: its layout tree, drawn by Workspace (layout frontend-and-plugins.md 2.6). The tree starts as the
// section's one panel, and the panel menu changes it for this window only.

import { useStore } from "@rastai/plugin";

import { useShell } from "../app/shell-context.js";
import { layoutOf } from "../state/layouts.js";
import { Workspace } from "./Workspace.jsx";

/**
 * @param {object} props
 * @param {string} props.sectionId
 */
export function SectionPage({ sectionId }) {
  const shell = useShell();
  const layouts = useStore(shell.stores.layouts);
  const node = layoutOf(layouts, sectionId, panelIdOf(sectionId));
  return <Workspace node={node} sectionId={sectionId} />;
}

/**
 * The panel a section's body is registered under (plugin-host/core-sections.js).
 * @param {string} sectionId
 * @returns {string}
 */
export function panelIdOf(sectionId) {
  return `core.${sectionId}`;
}
