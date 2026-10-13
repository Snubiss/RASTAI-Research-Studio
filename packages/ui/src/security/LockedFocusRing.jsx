// The focus ring of the security UI, drawn by the security components themselves (ui-design.md 5.1; layout
// frontend-and-plugins.md 2.10). It is a style element beside the focusable parts, so the ring is there even where
// components.css isn't loaded, and it takes its color from a locked token: no theme can change it.

import { LOCKED_FOCUS_RING_CSS } from "./locked-styles.js";

export function LockedFocusRing() {
  return <style>{LOCKED_FOCUS_RING_CSS}</style>;
}
