// The contrast rules of layout frontend-and-plugins.md 2.9 (WCAG 2.2 AA: 4.5:1 for text, 3:1 for UI parts, chart
// marks and the focus ring) on both built-in themes, with and without the colorblind palette, and on the locked
// security-UI colors. The colorblind palette must also stay distinguishable under simulated protanopia and
// deuteranopia (layout 4.5).

import { describe, expect, test } from "vitest";

import { BUILT_IN_THEMES } from "./built-in-themes.js";
import { LOCKED_TOKENS } from "./locked.js";
import { COLORBLIND_COLORS } from "./palettes.js";
import { builtInTheme } from "./resolve-tokens.js";

/** @typedef {import("./token-types.js").ThemeOptions} ThemeOptions */

const TEXT_CONTRAST = 4.5;
const UI_CONTRAST = 3;
const BASES = /** @type {const} */ (["light", "dark"]);
/** @type {ThemeOptions} */
const PLAIN = { density: "comfortable", reducedMotion: false, colorblind: false };
/** @type {ThemeOptions} */
const COLORBLIND = { density: "comfortable", reducedMotion: false, colorblind: true };
const SERIES_ROLES = ["series_1", "series_2", "series_3", "series_4", "series_5", "series_6", "series_7", "series_8"];
const PANEL_BACKGROUNDS = ["background", "surface", "surface_raised"];

// Each rule: the foreground roles, the background roles they sit on, and the ratio they need.
const ROLE_RULES = [
  { foregrounds: ["text"], backgrounds: [...PANEL_BACKGROUNDS, "surface_sunken"], ratio: TEXT_CONTRAST },
  { foregrounds: ["text_muted"], backgrounds: [...PANEL_BACKGROUNDS, "surface_sunken"], ratio: TEXT_CONTRAST },
  { foregrounds: ["text"], backgrounds: ["selection"], ratio: TEXT_CONTRAST },
  { foregrounds: ["text_on_accent"], backgrounds: ["accent", "accent_hover", "accent_active"], ratio: TEXT_CONTRAST },
  {
    foregrounds: ["accent", "focus_ring", "border_strong", "success"],
    backgrounds: PANEL_BACKGROUNDS,
    ratio: UI_CONTRAST,
  },
  { foregrounds: SERIES_ROLES, backgrounds: ["surface"], ratio: UI_CONTRAST },
  {
    foregrounds: ["sequential_high", "diverging_negative", "diverging_positive"],
    backgrounds: ["surface"],
    ratio: UI_CONTRAST,
  },
  { foregrounds: ["canvas_label"], backgrounds: ["canvas_background", "canvas_node"], ratio: TEXT_CONTRAST },
  {
    // A node is told from the canvas by its border, so its fill needs no ratio of its own.
    foregrounds: ["canvas_node_border", "canvas_node_selected", "canvas_edge", "canvas_edge_active", "canvas_port"],
    backgrounds: ["canvas_background"],
    ratio: UI_CONTRAST,
  },
  { foregrounds: ["canvas_node_border"], backgrounds: ["canvas_node"], ratio: UI_CONTRAST },
];
// The locked colors: security text on the security surface and on the egress badge, and the severity colors (a
// diagnostic's label and edge) on the theme's own panel backgrounds.
const LOCKED_RULES = [
  { foregrounds: ["security-text", "security-danger"], backgrounds: ["security-surface"], ratio: TEXT_CONTRAST },
  { foregrounds: ["egress-text"], backgrounds: ["egress"], ratio: TEXT_CONTRAST },
];
const SEVERITY_KEYS = ["severity-error", "severity-warning", "severity-info"];

// Machado, Oliveira and Fernandes (2009), severity 1.0: linear-RGB matrices that simulate each color vision
// deficiency.
const SIMULATIONS = {
  normal: [
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ],
  protanopia: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deuteranopia: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
};
// The smallest CIE76 color difference allowed between two colorblind series colors in any simulation. About 2.3 is
// "just noticeable"; 20 keeps lines and bars easy to tell apart at a glance.
const MIN_COLOR_DIFFERENCE = 20;
const D65_WHITE = [0.95047, 1, 1.08883];

/**
 * @param {string} hex   "#rrggbb"
 * @returns {number[]}   linear RGB, 0 to 1
 */
function linearRgb(hex) {
  return [1, 3, 5].map((start) => {
    const channel = Number.parseInt(hex.slice(start, start + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
}

/**
 * WCAG 2 relative luminance.
 * @param {string} hex
 * @returns {number}
 */
function luminance(hex) {
  const [red, green, blue] = linearRgb(hex);
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/**
 * WCAG 2 contrast ratio, from 1 to 21.
 * @param {string} first
 * @param {string} second
 * @returns {number}
 */
function contrastRatio(first, second) {
  const lighter = Math.max(luminance(first), luminance(second));
  const darker = Math.min(luminance(first), luminance(second));
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * CIELAB of a linear-RGB color under the D65 white point.
 * @param {number[]} rgb
 * @returns {number[]}
 */
function lab(rgb) {
  const [red, green, blue] = rgb;
  const xyz = [
    (0.4124 * red + 0.3576 * green + 0.1805 * blue) / D65_WHITE[0],
    (0.2126 * red + 0.7152 * green + 0.0722 * blue) / D65_WHITE[1],
    (0.0193 * red + 0.1192 * green + 0.9505 * blue) / D65_WHITE[2],
  ];
  const [fx, fy, fz] = xyz.map((value) => (value > 0.008856 ? Math.cbrt(value) : 7.787 * value + 16 / 116));
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/**
 * How a color looks under a simulation, as CIELAB.
 * @param {string} hex
 * @param {number[][]} matrix
 * @returns {number[]}
 */
function simulatedLab(hex, matrix) {
  const rgb = linearRgb(hex);
  const simulated = matrix.map((row) => {
    const value = row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2];
    return Math.min(1, Math.max(0, value));
  });
  return lab(simulated);
}

/**
 * Every failing pair of a rule set, as readable lines; an empty list passes.
 * @param {Array<{ foregrounds: string[], backgrounds: string[], ratio: number }>} rules
 * @param {Object<string, string>} foregroundColors
 * @param {Object<string, string>} backgroundColors
 * @returns {string[]}
 */
function contrastFailures(rules, foregroundColors, backgroundColors) {
  const failures = [];
  for (const rule of rules) {
    for (const foreground of rule.foregrounds) {
      for (const background of rule.backgrounds) {
        const ratio = contrastRatio(foregroundColors[foreground], backgroundColors[background]);
        if (ratio < rule.ratio) {
          failures.push(`${foreground} on ${background}: ${ratio.toFixed(2)} < ${rule.ratio}`);
        }
      }
    }
  }
  return failures;
}

describe("contrast", () => {
  for (const base of BASES) {
    test(`${base}: every role pair meets its ratio, plain and colorblind`, () => {
      for (const options of [PLAIN, COLORBLIND]) {
        const colors = builtInTheme(base, options).colors;
        expect(contrastFailures(ROLE_RULES, colors, colors)).toEqual([]);
      }
    });

    test(`${base}: the locked colors meet their ratios`, () => {
      const locked = LOCKED_TOKENS[base];
      expect(contrastFailures(LOCKED_RULES, locked, locked)).toEqual([]);
      const severityRule = [{ foregrounds: SEVERITY_KEYS, backgrounds: PANEL_BACKGROUNDS, ratio: TEXT_CONTRAST }];
      expect(contrastFailures(severityRule, locked, BUILT_IN_THEMES[base].colors)).toEqual([]);
      const badgeRule = [{ foregrounds: ["egress"], backgrounds: PANEL_BACKGROUNDS, ratio: UI_CONTRAST }];
      expect(contrastFailures(badgeRule, locked, BUILT_IN_THEMES[base].colors)).toEqual([]);
    });
  }

  test("the checker itself: black on white is 21, a color on itself is 1", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#336699", "#336699")).toBe(1);
  });
});

describe("colorblind palette", () => {
  for (const base of BASES) {
    test(`${base}: the series colors stay apart under protanopia and deuteranopia`, () => {
      const series = SERIES_ROLES.map((role) => COLORBLIND_COLORS[base][role]);
      for (const [name, matrix] of Object.entries(SIMULATIONS)) {
        const labs = series.map((color) => simulatedLab(color, matrix));
        for (let first = 0; first < labs.length; first += 1) {
          for (let second = first + 1; second < labs.length; second += 1) {
            const difference = Math.hypot(
              labs[first][0] - labs[second][0],
              labs[first][1] - labs[second][1],
              labs[first][2] - labs[second][2],
            );
            const pair = `${name}: ${series[first]} and ${series[second]}`;
            expect(difference, pair).toBeGreaterThanOrEqual(MIN_COLOR_DIFFERENCE);
          }
        }
      }
    });

    test(`${base}: the diverging map is blue and orange, not red and green`, () => {
      const negative = linearRgb(COLORBLIND_COLORS[base].diverging_negative);
      const positive = linearRgb(COLORBLIND_COLORS[base].diverging_positive);
      expect(negative[2]).toBeGreaterThan(negative[0]);
      expect(positive[0]).toBeGreaterThan(positive[2]);
    });
  }

  test("the simulation itself: red and green collapse under deuteranopia", () => {
    const red = simulatedLab("#cc3333", SIMULATIONS.deuteranopia);
    const green = simulatedLab("#669933", SIMULATIONS.deuteranopia);
    expect(Math.hypot(red[0] - green[0], red[1] - green[1], red[2] - green[2])).toBeLessThan(MIN_COLOR_DIFFERENCE);
  });
});
