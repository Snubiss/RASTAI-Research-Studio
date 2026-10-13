// The theme state (layout frontend-and-plugins.md 2.10; design 4). It picks the built-in theme from ui.theme and the
// OS color scheme, resolves it with ui.density, ui.reduced_motion and ui.colorblind_palette, and writes the --rr-*
// variables. `system` follows the OS with no command: a change of prefers-color-scheme re-resolves here.
// The locked security tokens never reach a plugin or a component through the theme (withoutLockedTokens).

import { applyThemeTokens, builtInTheme, LOCKED_TOKENS } from "@rastai/ui";

import { DEFAULT_DENSITY, DEFAULT_THEME_SETTING } from "../constants.js";
import { readSetting } from "./settings.js";

/** @typedef {import("@rastai/ui").ThemeTokens} ThemeTokens */
/** @typedef {import("@rastai/ui").BuiltInThemeName} BuiltInThemeName */

const SYSTEM = "system";
const DARK = "dark";
const HIGH_CONTRAST = "high_contrast";
const COMPACT = "compact";
const LIGHT = "light";

/**
 * The built-in theme a ui.theme value names. `system` takes the OS scheme; the high-contrast theme is its own name.
 * @param {string} themeSetting
 * @param {boolean} prefersDark   the OS color scheme
 * @returns {BuiltInThemeName}
 */
export function themeNameFor(themeSetting, prefersDark) {
  if (themeSetting === HIGH_CONTRAST) {
    return HIGH_CONTRAST;
  }
  if (themeSetting === DARK) {
    return DARK;
  }
  if (themeSetting === LIGHT) {
    return LIGHT;
  }
  return prefersDark ? DARK : LIGHT;
}

/**
 * The resolved tokens for the settings and the OS. A value the app doesn't know falls back to `system`.
 * @param {any} values   the settings.get result's `values`
 * @param {{ prefersDark: boolean, prefersReducedMotion: boolean }} os
 * @returns {ThemeTokens}
 */
export function themeTokensFor(values, os) {
  const setting = readSetting(values, "ui.theme", DEFAULT_THEME_SETTING);
  const density = readSetting(values, "ui.density", DEFAULT_DENSITY) === COMPACT ? COMPACT : "comfortable";
  const options = {
    density: /** @type {"comfortable" | "compact"} */ (density),
    reducedMotion: readSetting(values, "ui.reduced_motion", false) === true || os.prefersReducedMotion,
    colorblind: readSetting(values, "ui.colorblind_palette", false) === true,
  };
  return builtInTheme(themeNameFor(setting, os.prefersDark), options);
}

/**
 * The tokens with every locked security color taken out. The host hands plugins these, so a theme can't reach the
 * locked set, even one a future theme file names (contracts 5.6; layout 2.10).
 * @param {ThemeTokens} tokens
 * @returns {ThemeTokens}
 */
export function withoutLockedTokens(tokens) {
  const lockedNames = new Set([...Object.keys(LOCKED_TOKENS.light), ...Object.keys(LOCKED_TOKENS.dark)]);
  /** @type {Object<string, string>} */
  const colors = {};
  for (const [role, value] of Object.entries(tokens.colors)) {
    if (!lockedNames.has(role)) {
      colors[role] = value;
    }
  }
  return { ...tokens, colors };
}

/**
 * Writes the tokens to the page's root: the --rr-* variables, then the locked ones, as applyThemeTokens does.
 * @param {ThemeTokens} tokens
 */
export function applyTheme(tokens) {
  applyThemeTokens(document.documentElement, tokens);
}

/**
 * Calls onChange whenever the OS color scheme changes, so `system` follows it without a command.
 * @param {{ matches: boolean, addEventListener: (type: string, listener: () => void) => void, removeEventListener: (type: string, listener: () => void) => void }} query
 *   a MediaQueryList for (prefers-color-scheme: dark)
 * @param {(prefersDark: boolean) => void} onChange
 * @returns {() => void} stops listening
 */
export function watchColorScheme(query, onChange) {
  const listener = () => onChange(query.matches);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}
