// The built-in light and dark themes (layout frontend-and-plugins.md 2.10; contracts 5.6). Each is a complete
// rastai.theme/1 file: every color role is set, so any theme's missing roles can come from its base. The files are
// plain JSON so rastai-formats reads the same bytes (crates/rastai-formats/tests/theme_tokens.rs validates both
// through read_document). Dark's canvas_background is the spike canvas's #101318, the color the renderers used to
// hard-code (spike/canvas).

import DARK_THEME from "./dark.theme.json" with { type: "json" };
import LIGHT_THEME from "./light.theme.json" with { type: "json" };

/** @typedef {typeof LIGHT_THEME} BuiltInTheme */

/** The two built-in themes by base. */
export const BUILT_IN_THEMES = { light: LIGHT_THEME, dark: DARK_THEME };

/**
 * Every color role key, in the theme file's order. The light theme holds them all, and tokens.test.js checks this
 * list against the generated schema, so a role added to rastai.theme/1 fails the test until both themes set it.
 * @type {string[]}
 */
export const COLOR_ROLES = Object.keys(LIGHT_THEME.colors);
