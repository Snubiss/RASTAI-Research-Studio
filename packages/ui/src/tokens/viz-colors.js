// vizColors(tokens): the plain color data every @rastai/viz chart takes through setColors (contracts 5.6; layout
// frontend-and-plugins.md 2.10 step 5). Charts sit on a panel, so their background is the surface role.
//
// The VizColors type belongs to @rastai/viz, which arrives in M4. Until then its shape is written once, here; M4
// moves the @typedef into @rastai/viz and this file names it with a JSDoc type import (contracts 2.4).

import { labelFont } from "./canvas-colors.js";
import { hexToRgba } from "./color-math.js";

/** @typedef {import("./token-types.js").ThemeTokens} ThemeTokens */
/** @typedef {import("./canvas-colors.js").GlyphFont} GlyphFont */

const SERIES_ROLES = ["series_1", "series_2", "series_3", "series_4", "series_5", "series_6", "series_7", "series_8"];

/**
 * RGBA arrays (0 to 1) for charts. `sequential` is [low, high]; `diverging` is [negative, zero, positive].
 * @typedef {object} VizColors
 * @property {number[][]} series
 * @property {number[]} background
 * @property {number[]} axis
 * @property {number[]} grid
 * @property {number[]} label
 * @property {number[][]} sequential
 * @property {number[][]} diverging
 * @property {GlyphFont} labelFont
 */

/**
 * @param {ThemeTokens} tokens
 * @returns {VizColors}
 */
export function vizColors(tokens) {
  const colors = tokens.colors;
  return {
    series: SERIES_ROLES.map((role) => hexToRgba(colors[role])),
    background: hexToRgba(colors.surface),
    axis: hexToRgba(colors.border_strong),
    grid: hexToRgba(colors.divider),
    label: hexToRgba(colors.text_muted),
    sequential: [hexToRgba(colors.sequential_low), hexToRgba(colors.sequential_high)],
    diverging: [
      hexToRgba(colors.diverging_negative),
      hexToRgba(colors.diverging_zero),
      hexToRgba(colors.diverging_positive),
    ],
    labelFont: labelFont(tokens),
  };
}
