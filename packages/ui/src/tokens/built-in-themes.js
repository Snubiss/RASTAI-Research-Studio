// The built-in themes (layout frontend-and-plugins.md 2.10; contracts 5.6; ui-design.md 4): light, dark and the
// high-contrast theme. Each is a complete rastai.theme/1 file: every color role is set, so any theme's missing roles
// can come from its base. The files are plain JSON so rastai-formats reads the same bytes
// (crates/rastai-formats/tests/theme_tokens.rs validates all three through read_document). Dark's canvas_background is
// the spike canvas's #101318 before the design's values; the design's dark canvas is #0d0f13.
// High contrast is keyed by its own name, not by its base: its base is "dark", so it takes the dark locked set.

import DARK_THEME from "./dark.theme.json" with { type: "json" };
import HIGH_CONTRAST_THEME from "./high_contrast.theme.json" with { type: "json" };
import LIGHT_THEME from "./light.theme.json" with { type: "json" };

/** @typedef {typeof LIGHT_THEME} BuiltInTheme */

/** The built-in themes by name (the `ui.theme` value, or "light" and "dark" for a base). */
export const BUILT_IN_THEMES = { light: LIGHT_THEME, dark: DARK_THEME, high_contrast: HIGH_CONTRAST_THEME };

/**
 * Every color role key, in the theme file's order. The light theme holds them all, and tokens.test.js checks this
 * list against the generated schema, so a role added to rastai.theme/1 fails the test until both themes set it.
 * @type {string[]}
 */
export const COLOR_ROLES = Object.keys(LIGHT_THEME.colors);
