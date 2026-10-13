// applyThemeTokens(root, tokens): writes the --rr-* CSS variables, the public surface every component and plugin
// UI reads (contracts 5.6; layout frontend-and-plugins.md 2.10 step 4). The locked --rr-locked-* variables are
// written last and come only from LOCKED_TOKENS, so nothing in a tokens object can change them.

import { COLOR_ROLES } from "./built-in-themes.js";
import { LOCKED_PREFIX, lockedTokensFor } from "./locked.js";

/** @typedef {import("./token-types.js").ThemeTokens} ThemeTokens */

/**
 * The part of an element applyThemeTokens writes to; an HTMLElement has it.
 * @typedef {object} StyleTarget
 * @property {{ setProperty: (name: string, value: string) => void }} style
 */

/** One CSS variable: [name, value]. @typedef {[string, string]} CssVariable */

const FONT_SIZE_KEYS = ["xs", "sm", "md", "lg", "xl"];
const FONT_WEIGHT_KEYS = ["regular", "medium", "bold"];
const LINE_HEIGHT_KEYS = ["body", "heading", "code"];
const RADIUS_KEYS = ["small", "medium", "large"];
// Lets the browser draw its own parts (scrollbars, native form controls) for the same light or dark base.
const COLOR_SCHEME_PROPERTY = "color-scheme";

/**
 * Sets every --rr-* variable from the tokens, then every --rr-locked-* variable from LOCKED_TOKENS.
 * @param {StyleTarget} root   usually document.documentElement
 * @param {ThemeTokens} tokens
 */
export function applyThemeTokens(root, tokens) {
  for (const [name, value] of themeVariables(tokens)) {
    root.style.setProperty(name, value);
  }
  root.style.setProperty(COLOR_SCHEME_PROPERTY, lockedBase(tokens));
  for (const [name, value] of lockedVariables(tokens)) {
    root.style.setProperty(name, value);
  }
}

/**
 * Every themeable variable, in the order applyThemeTokens writes them. Colors come only from the role keys, so a
 * stray key in `tokens.colors` is never written.
 * @param {ThemeTokens} tokens
 * @returns {CssVariable[]}
 */
export function themeVariables(tokens) {
  return [
    ...colorVariables(tokens),
    ...typographyVariables(tokens),
    ...spacingVariables(tokens),
    ...RADIUS_KEYS.map((key) => cssVariable(`--rr-radius-${key}`, pixels(readNumber(tokens.radiusPx, key)))),
    cssVariable("--rr-motion-fast", `${tokens.motion.fastMs}ms`),
    cssVariable("--rr-motion-slow", `${tokens.motion.slowMs}ms`),
    cssVariable("--rr-motion-easing", tokens.motion.easing),
    cssVariable("--rr-focus-ring-width", pixels(tokens.focusRing.widthPx)),
    cssVariable("--rr-focus-ring-offset", pixels(tokens.focusRing.offsetPx)),
    cssVariable("--rr-elevation-1", tokens.elevation.level1),
    cssVariable("--rr-elevation-2", tokens.elevation.level2),
    cssVariable("--rr-elevation-3", tokens.elevation.level3),
  ];
}

/**
 * The locked variables for the tokens' base. Only `base` is read from the tokens.
 * @param {ThemeTokens} tokens
 * @returns {CssVariable[]}
 */
export function lockedVariables(tokens) {
  const locked = lockedTokensFor(tokens.base);
  return Object.keys(locked).map((key) => cssVariable(`${LOCKED_PREFIX}${key}`, locked[key]));
}

/**
 * `surface_raised` -> `--rr-color-surface-raised`.
 * @param {string} role
 * @returns {string}
 */
export function colorVariableName(role) {
  return `--rr-color-${role.replaceAll("_", "-")}`;
}

/**
 * @param {ThemeTokens} tokens
 * @returns {CssVariable[]}
 */
function colorVariables(tokens) {
  return COLOR_ROLES.map((role) => cssVariable(colorVariableName(role), tokens.colors[role]));
}

/**
 * @param {ThemeTokens} tokens
 * @returns {CssVariable[]}
 */
function typographyVariables(tokens) {
  const typography = tokens.typography;
  return [
    cssVariable("--rr-font-ui", typography.fontFamilyUi),
    cssVariable("--rr-font-mono", typography.fontFamilyMono),
    ...FONT_WEIGHT_KEYS.map((key) =>
      cssVariable(`--rr-font-weight-${key}`, String(readNumber(typography.fontWeights, key))),
    ),
    ...FONT_SIZE_KEYS.map((key) =>
      cssVariable(`--rr-font-size-${key}`, pixels(readNumber(typography.fontSizesPx, key))),
    ),
    ...LINE_HEIGHT_KEYS.map((key) =>
      cssVariable(`--rr-line-height-${key}`, String(readNumber(typography.lineHeights, key))),
    ),
  ];
}

/**
 * --rr-space-<n> is step n of the active density's scale.
 * @param {ThemeTokens} tokens
 * @returns {CssVariable[]}
 */
function spacingVariables(tokens) {
  return tokens.spacingPx.map((size, step) => cssVariable(`--rr-space-${step}`, pixels(size)));
}

/**
 * @param {ThemeTokens} tokens
 * @returns {"light" | "dark"}
 */
function lockedBase(tokens) {
  return tokens.base === "dark" ? "dark" : "light";
}

/**
 * Reads a numeric field by name from one of the small token groups.
 * @param {Object<string, number>} group
 * @param {string} key
 * @returns {number}
 */
function readNumber(group, key) {
  return group[key];
}

/**
 * @param {number} size
 * @returns {string}
 */
function pixels(size) {
  return `${size}px`;
}

/**
 * @param {string} name
 * @param {string} value
 * @returns {CssVariable}
 */
function cssVariable(name, value) {
  return [name, value];
}
