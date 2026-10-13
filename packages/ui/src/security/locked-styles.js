// The look of the security UI (owner decision N7; layout frontend-and-plugins.md 2.10). Colors come only from the
// locked tokens, and the font, sizes and spacing are fixed here instead of read from the theme, so no theme can
// shrink, hide or recolor a security component. The styles are inline: an inline style beats any stylesheet rule
// without !important, and the components work even where components.css isn't loaded.

import { LOCKED_PREFIX, LOCKED_TOKENS } from "../tokens/locked.js";

// Fixed on purpose: these never follow the theme's typography or density tokens.
const SECURITY_FONT = 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';
const BODY_FONT_SIZE = "14px";
const SMALL_FONT_SIZE = "12px";
const HEADING_FONT_SIZE = "18px";
const LINE_HEIGHT = 1.5;
const BOLD = 700;
// WCAG 2.2's minimum target size (layout 2.9).
const MIN_TARGET_SIZE = "24px";

/**
 * `var(--rr-locked-<key>, <light value>)`: the light set is the fallback before applyThemeTokens has run.
 * @param {string} key   a LOCKED_TOKENS key, such as "security-text"
 * @returns {string}
 */
export function lockedColor(key) {
  return `var(${LOCKED_PREFIX}${key}, ${LOCKED_TOKENS.light[key]})`;
}

/** @typedef {import("react").CSSProperties} CSSProperties */

// Every security style starts from this, so no element inherits the theme's font, size or spacing.
/** @type {CSSProperties} */
const FIXED_TEXT = {
  fontFamily: SECURITY_FONT,
  fontSize: BODY_FONT_SIZE,
  fontWeight: "normal",
  fontStyle: "normal",
  lineHeight: LINE_HEIGHT,
  letterSpacing: "normal",
  textTransform: "none",
};

/** @type {CSSProperties} */
export const PANEL_STYLE = {
  ...FIXED_TEXT,
  background: lockedColor("security-surface"),
  color: lockedColor("security-text"),
  border: `2px solid ${lockedColor("security-text")}`,
  borderRadius: "8px",
  padding: "16px",
  opacity: 1,
  visibility: "visible",
};

/** @type {CSSProperties} */
export const HEADING_STYLE = {
  ...FIXED_TEXT,
  margin: "0 0 8px",
  fontSize: HEADING_FONT_SIZE,
  fontWeight: BOLD,
  color: lockedColor("security-text"),
};

/** @type {CSSProperties} */
export const TEXT_STYLE = {
  ...FIXED_TEXT,
  margin: "0 0 8px",
  color: lockedColor("security-text"),
};

/** @type {CSSProperties} */
export const DANGER_TEXT_STYLE = {
  ...FIXED_TEXT,
  margin: "0 0 8px",
  color: lockedColor("security-danger"),
  fontWeight: BOLD,
};

/** @type {CSSProperties} */
export const LIST_STYLE = {
  ...FIXED_TEXT,
  margin: "0 0 8px",
  paddingLeft: "20px",
  color: lockedColor("security-text"),
};

/** @type {CSSProperties} */
export const BUTTON_STYLE = {
  ...FIXED_TEXT,
  minHeight: MIN_TARGET_SIZE,
  padding: "4px 12px",
  marginRight: "8px",
  border: `2px solid ${lockedColor("security-text")}`,
  borderRadius: "4px",
  background: lockedColor("security-surface"),
  color: lockedColor("security-text"),
  fontWeight: BOLD,
  cursor: "pointer",
};

/** @type {CSSProperties} */
export const BADGE_STYLE = {
  ...FIXED_TEXT,
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  minHeight: MIN_TARGET_SIZE,
  padding: "2px 10px",
  border: `1px solid ${lockedColor("egress-text")}`,
  borderRadius: "12px",
  background: lockedColor("egress"),
  color: lockedColor("egress-text"),
  fontSize: SMALL_FONT_SIZE,
  fontWeight: BOLD,
  cursor: "pointer",
  opacity: 1,
  visibility: "visible",
};

/** @type {CSSProperties} */
export const SCREEN_STYLE = {
  ...FIXED_TEXT,
  position: "fixed",
  inset: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: lockedColor("security-surface"),
  color: lockedColor("security-text"),
  zIndex: 2147483647,
};

/** @type {CSSProperties} */
export const CODE_STYLE = {
  ...FIXED_TEXT,
  fontFamily: "ui-monospace, monospace",
  fontSize: SMALL_FONT_SIZE,
  color: lockedColor("security-text"),
};
