// Renders its children only in Advanced mode (layout frontend-and-plugins.md 2.4). There is no BasicOnly, so
// Advanced can only add: a Basic control is never hidden in Advanced (spec §7). Place it after or beside Basic
// blocks, never around them, and keep the values of the controls inside it in the panel's store, not in their own
// state, so switching modes keeps them.

import { useMode } from "./use-mode.js";

/** @typedef {import("./mode-types.js").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {ReactNode} props.children
 */
export function AdvancedOnly({ children }) {
  const mode = useMode();
  if (mode !== "advanced") {
    return null;
  }
  return <>{children}</>;
}
