// builtInTheme(base, options): a built-in theme resolved with the user's density, motion and colorblind settings
// (layout frontend-and-plugins.md 2.10 step 3; contracts 5.6). Plugin themes (M8) go through the same
// resolveTheme, with their missing roles taken from their base.

import { BUILT_IN_THEMES, COLOR_ROLES } from "./built-in-themes.js";
import { COLORBLIND_COLORS } from "./palettes.js";

/** @typedef {import("./token-types.js").ThemeTokens} ThemeTokens */
/** @typedef {import("./token-types.js").ThemeOptions} ThemeOptions */
/** @typedef {import("./built-in-themes.js").BuiltInTheme} BuiltInTheme */

const COMPACT_DENSITY = "compact";

/**
 * @param {"light" | "dark"} base
 * @param {ThemeOptions} options
 * @returns {ThemeTokens}
 */
export function builtInTheme(base, options) {
  return resolveTheme(builtInThemeFor(base), options);
}

/**
 * Resolves a rastai.theme/1 document: its colors over its base's, then the options. Only the role keys are read
 * from `colors`, so a stray key (a locked name, a typo) never becomes a variable.
 * @param {BuiltInTheme} theme
 * @param {ThemeOptions} options
 * @returns {ThemeTokens}
 */
export function resolveTheme(theme, options) {
  const base = theme.base === "dark" ? "dark" : "light";
  return {
    name: theme.name,
    base,
    colors: resolveColors(theme, base, options.colorblind),
    typography: {
      fontFamilyUi: theme.typography.font_family_ui,
      fontFamilyMono: theme.typography.font_family_mono,
      fontWeights: { ...theme.typography.font_weights },
      fontSizesPx: { ...theme.typography.font_sizes_px },
      lineHeights: { ...theme.typography.line_heights },
    },
    spacingPx: spacingFor(theme, options.density),
    radiusPx: { ...theme.radius_px },
    motion: {
      fastMs: options.reducedMotion ? 0 : theme.motion.fast_ms,
      slowMs: options.reducedMotion ? 0 : theme.motion.slow_ms,
      easing: theme.motion.easing,
    },
    focusRing: { widthPx: theme.focus_ring.width_px, offsetPx: theme.focus_ring.offset_px },
    elevation: {
      level1: theme.elevation.level_1,
      level2: theme.elevation.level_2,
      level3: theme.elevation.level_3,
    },
  };
}

/**
 * The built-in theme for a base; anything but "dark" is light.
 * @param {string} base
 * @returns {BuiltInTheme}
 */
function builtInThemeFor(base) {
  if (base === "dark") {
    return BUILT_IN_THEMES.dark;
  }
  return BUILT_IN_THEMES.light;
}

/**
 * Every role: the theme's own color, else its base's. The colorblind set replaces the series and diverging roles
 * last, so the setting works with any theme.
 * @param {BuiltInTheme} theme
 * @param {"light" | "dark"} base
 * @param {boolean} colorblind
 * @returns {Object<string, string>}
 */
function resolveColors(theme, base, colorblind) {
  /** @type {Object<string, string>} */
  const themeColors = theme.colors;
  /** @type {Object<string, string>} */
  const baseColors = BUILT_IN_THEMES[base].colors;
  /** @type {Object<string, string>} */
  const colors = {};
  for (const role of COLOR_ROLES) {
    colors[role] = themeColors[role] ?? baseColors[role];
  }
  if (colorblind) {
    Object.assign(colors, COLORBLIND_COLORS[base]);
  }
  return colors;
}

/**
 * @param {BuiltInTheme} theme
 * @param {string} density
 * @returns {number[]}
 */
function spacingFor(theme, density) {
  if (density === COMPACT_DENSITY) {
    return [...theme.density.compact.spacing_px];
  }
  return [...theme.density.comfortable.spacing_px];
}
