// canvasColors(tokens): the plain color data the graph renderer takes through setColors (contracts 5.6; layout
// frontend-and-plugins.md 2.10 step 5). Renderers never pick colors and never see CSS.
//
// The CanvasColors type belongs to @rastai/canvas, which arrives in M4. Until then its shape is written once, here;
// M4 moves the @typedef into @rastai/canvas and this file names it with a JSDoc type import (contracts 2.4).

import { hexToRgba } from "./color-math.js";

/** @typedef {import("./token-types.js").ThemeTokens} ThemeTokens */

/**
 * The font the glyph atlas draws labels with (createGlyphAtlas(gl, font), contracts 5.6).
 * @typedef {object} GlyphFont
 * @property {string} family
 * @property {number} weight
 * @property {number} sizePx
 */

/**
 * RGBA arrays (0 to 1) per canvas role, plus the label font.
 * @typedef {object} CanvasColors
 * @property {number[]} background
 * @property {number[]} grid
 * @property {number[]} node
 * @property {number[]} nodeBorder
 * @property {number[]} nodeSelected
 * @property {number[]} edge
 * @property {number[]} edgeActive
 * @property {number[]} label
 * @property {number[]} port
 * @property {GlyphFont} labelFont
 */

/**
 * @param {ThemeTokens} tokens
 * @returns {CanvasColors}
 */
export function canvasColors(tokens) {
  const colors = tokens.colors;
  return {
    background: hexToRgba(colors.canvas_background),
    grid: hexToRgba(colors.canvas_grid),
    node: hexToRgba(colors.canvas_node),
    nodeBorder: hexToRgba(colors.canvas_node_border),
    nodeSelected: hexToRgba(colors.canvas_node_selected),
    edge: hexToRgba(colors.canvas_edge),
    edgeActive: hexToRgba(colors.canvas_edge_active),
    label: hexToRgba(colors.canvas_label),
    port: hexToRgba(colors.canvas_port),
    labelFont: labelFont(tokens),
  };
}

/**
 * Canvas and chart labels use the interface font at the small size.
 * @param {ThemeTokens} tokens
 * @returns {GlyphFont}
 */
export function labelFont(tokens) {
  return {
    family: tokens.typography.fontFamilyUi,
    weight: tokens.typography.fontWeights.regular,
    sizePx: tokens.typography.fontSizesPx.sm,
  };
}
