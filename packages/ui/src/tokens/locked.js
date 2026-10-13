// The locked security-UI tokens (owner decision N7; contracts 5.6; layout frontend-and-plugins.md 2.10). The egress
// badge, the trust prompt, the permission list, the signed-out screen and the error severity colors use only these.
// No theme, plugin or override reaches them: applyThemeTokens sets them last, from here, chosen by the theme's base
// alone, and a rastai.theme/1 file that names one fails validation (RRS-E1003). Keys are the CSS variable names
// without the `--rr-locked-` prefix.

/** The CSS variable prefix of every locked token. */
export const LOCKED_PREFIX = "--rr-locked-";

/**
 * One light and one dark set. palettes.test.js checks their contrast against the built-in backgrounds.
 * @type {{ light: Object<string, string>, dark: Object<string, string> }}
 */
export const LOCKED_TOKENS = {
  light: {
    "severity-error": "#b42318",
    "severity-warning": "#9a6700",
    "severity-info": "#0a5cc2",
    egress: "#7a4b00",
    "egress-text": "#ffffff",
    "security-surface": "#fffbea",
    "security-text": "#1a1300",
    "security-danger": "#b42318",
  },
  dark: {
    "severity-error": "#ff6b6b",
    "severity-warning": "#f2c94c",
    "severity-info": "#6cb6ff",
    egress: "#ffd23f",
    "egress-text": "#1a1300",
    "security-surface": "#1d1a10",
    "security-text": "#fff4d6",
    "security-danger": "#ff8a80",
  },
};

/**
 * The locked set for a base. Anything but "dark" gets the light set, so a broken tokens object still draws the
 * security UI with real colors.
 * @param {string} base
 * @returns {Object<string, string>}
 */
export function lockedTokensFor(base) {
  if (base === "dark") {
    return LOCKED_TOKENS.dark;
  }
  return LOCKED_TOKENS.light;
}
