// One panel: its title bar (PanelChrome, with the Basic/Advanced override and the panel menu), the panel's mode, the
// host it reads through, and an error boundary so a crash stays in this panel (layout frontend-and-plugins.md 2.4, 2.6,
// 2.7; design 3.3).

import { PluginHostProvider, useStore } from "@rastai/plugin";
import { ModeProvider, PanelChrome } from "@rastai/ui";

import { ErrorBoundary } from "../app/ErrorBoundary.jsx";
import { useShell } from "../app/shell-context.js";
import { NotAvailableSection } from "../sections/NotAvailableSection.jsx";
import { panelById } from "../plugin-host/plugin-registry.js";
import { effectiveMode, sectionLabel } from "../state/mode.js";
import { layoutOf, setLayoutOf } from "../state/layouts.js";
import { defaultLayout, setModeOverride } from "./layout-tree.js";

/** @typedef {import("@rastai/ui").ModeOverride} ModeOverride */

const MENU_RESET = "reset";
// The panel menu's items (design 3.3). Only what does something is listed: every section has one panel in this version,
// so there is nothing to move beside and no tab to open. "Move to right/bottom", "New tab" and "Open in new window"
// come with the sections that have more than one panel, and with the second windows (M1-55).
const PANEL_MENU = [{ id: MENU_RESET, label: "Reset layout" }];

/**
 * @param {object} props
 * @param {string} props.sectionId
 * @param {string} props.panelId
 * @param {ModeOverride} props.modeOverride
 */
export function PanelFrame({ sectionId, panelId, modeOverride }) {
  const shell = useShell();
  const display = useStore(shell.stores.display);
  const mode = effectiveMode(display.mode, modeOverride);
  const panel = panelById(panelId, shell.loadedPluginIds);

  /** @param {(root: import("./layout-tree.js").LayoutNode) => import("./layout-tree.js").LayoutNode} change */
  function changeLayout(change) {
    const root = layoutOf(shell.stores.layouts.get(), sectionId, panelId);
    setLayoutOf(shell.stores.layouts, sectionId, change(root));
  }

  /** @param {string} itemId */
  function onMenuSelect(itemId) {
    if (itemId === MENU_RESET) {
      changeLayout(() => defaultLayout(panelId));
    }
  }

  return (
    <section className="rr-panel" aria-label={sectionLabel(sectionId, mode)}>
      <PanelChrome
        title={sectionLabel(sectionId, mode)}
        modeOverride={modeOverride}
        onModeOverrideChange={(value) => changeLayout((root) => setModeOverride(root, panelId, value))}
        menuItems={PANEL_MENU}
        onMenuSelect={onMenuSelect}
      />
      <ModeProvider mode={mode}>
        <PluginHostProvider host={shell.coreHost}>
          <ErrorBoundary panelId={panelId}>
            {panel === null ? <NotAvailableSection sectionId={sectionId} /> : panel.render({ params: {} })}
          </ErrorBoundary>
        </PluginHostProvider>
      </ModeProvider>
    </section>
  );
}
