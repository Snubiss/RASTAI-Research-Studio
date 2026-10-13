// The colorblind-safe colors ui.colorblind_palette swaps in (layout frontend-and-plugins.md 2.9 and 2.10): the
// eight series colors and blue/orange instead of red/green for the diverging map. Each base has its own set,
// because a color must keep 3:1 against its background. The sets start from the Okabe-Ito pair (blue #0072b2 and
// orange #d55e00 on light, sky blue #56b4e9 and orange #e69f00 on dark); the other six were picked to stay apart
// under simulated protanopia and deuteranopia. palettes.test.js checks both rules.

/**
 * Role key -> color, for the roles the colorblind setting replaces.
 * @type {{ light: Object<string, string>, dark: Object<string, string> }}
 */
export const COLORBLIND_COLORS = {
  light: {
    series_1: "#0072b2",
    series_2: "#d55e00",
    series_3: "#008888",
    series_4: "#882244",
    series_5: "#6666ee",
    series_6: "#aa8866",
    series_7: "#880000",
    series_8: "#0022ee",
    diverging_negative: "#0072b2",
    diverging_zero: "#eef1f4",
    diverging_positive: "#d55e00",
  },
  dark: {
    series_1: "#56b4e9",
    series_2: "#e69f00",
    series_3: "#aacc88",
    series_4: "#7a5cf0",
    series_5: "#c05050",
    series_6: "#aacccc",
    series_7: "#d42a0a",
    series_8: "#727296",
    diverging_negative: "#56b4e9",
    diverging_zero: "#2a2f38",
    diverging_positive: "#e69f00",
  },
};
