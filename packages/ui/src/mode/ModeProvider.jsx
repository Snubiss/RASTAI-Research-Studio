// Provides the effective mode to everything below: App renders one with ui.mode, and each PanelFrame renders
// another with the layout's mode_override (layout frontend-and-plugins.md 2.4).

import { ModeContext } from "./mode-context.js";

/** @typedef {import("./mode-types.js").UiMode} UiMode */
/** @typedef {import("./mode-types.js").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {UiMode} props.mode
 * @param {ReactNode} props.children
 */
export function ModeProvider({ mode, children }) {
  return <ModeContext.Provider value={mode}>{children}</ModeContext.Provider>;
}
