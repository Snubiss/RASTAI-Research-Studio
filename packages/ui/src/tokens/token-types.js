// The shapes of resolved theme tokens (layout frontend-and-plugins.md 4.2; contracts 5.6). JSDoc only, no code.
// A rastai.theme/1 file is snake_case (its type is `ThemeDocument`, generated into @rastai/formats); the resolved
// tokens here are camelCase because only JavaScript reads them. The color role keys are not re-typed: `colors` is
// keyed by the theme file's own role names, and the built-in themes hold every role (tokens.test.js checks them
// against the generated schema, schemas/rastai.theme/1.json).

/**
 * Resolved tokens: a theme plus the user's density, motion and colorblind settings.
 * @typedef {object} ThemeTokens
 * @property {string} name
 * @property {"light" | "dark"} base                 picks the locked set and the roles a theme leaves out
 * @property {Object<string, string>} colors          role key ("surface_raised") -> "#rrggbb"
 * @property {TypographyTokens} typography
 * @property {number[]} spacingPx                     the active density's scale; --rr-space-<n> is step n
 * @property {{ small: number, medium: number, large: number }} radiusPx
 * @property {{ fastMs: number, slowMs: number, easing: string }} motion
 * @property {{ widthPx: number, offsetPx: number }} focusRing
 * @property {{ level1: string, level2: string, level3: string }} elevation   CSS box-shadow values
 */

/**
 * @typedef {object} TypographyTokens
 * @property {string} fontFamilyUi
 * @property {string} fontFamilyMono
 * @property {{ regular: number, medium: number, bold: number }} fontWeights
 * @property {{ xs: number, sm: number, md: number, lg: number, xl: number }} fontSizesPx
 * @property {{ body: number, heading: number, code: number }} lineHeights
 */

/**
 * The user's settings that change the resolved tokens.
 * @typedef {object} ThemeOptions
 * @property {"comfortable" | "compact"} density   ui.density
 * @property {boolean} reducedMotion               ui.reduced_motion or the OS setting: both durations become 0
 * @property {boolean} colorblind                  ui.colorblind_palette: swaps the series and diverging colors
 */

export {};
