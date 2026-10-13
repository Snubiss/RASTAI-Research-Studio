// The token contract (contracts 5.6; layout frontend-and-plugins.md 2.10 and 4.5): every role key resolves in both
// built-in themes, applyThemeTokens writes every documented variable, and nothing in a tokens object reaches the
// locked variables.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { applyThemeTokens, colorVariableName, lockedVariables, themeVariables } from "./apply-theme.js";
import { BUILT_IN_THEMES, COLOR_ROLES } from "./built-in-themes.js";
import { canvasColors } from "./canvas-colors.js";
import { LOCKED_TOKENS } from "./locked.js";
import { COLORBLIND_COLORS } from "./palettes.js";
import { builtInTheme, resolveTheme } from "./resolve-tokens.js";
import { vizColors } from "./viz-colors.js";

/** @typedef {import("./token-types.js").ThemeTokens} ThemeTokens */
/** @typedef {import("./token-types.js").ThemeOptions} ThemeOptions */
/** @typedef {import("./built-in-themes.js").BuiltInTheme} BuiltInTheme */
/** @typedef {import("./token-types.js").BuiltInThemeName} BuiltInThemeName */

// The generated JSON Schema of rastai.theme/1, which rastai-formats writes from its own types (`just codegen`).
const THEME_SCHEMA_FILE = new URL("../../../../schemas/rastai.theme/1.json", import.meta.url);
// The Rust formats' fixture that sets every color role; it is the one file the Rust tests read as a full theme.
const EVERY_ROLE_FIXTURE = new URL(
  "../../../../fixtures/documents/rastai.theme/valid/every_role.theme.json",
  import.meta.url,
);
const HEX_COLOR = /^#[0-9a-f]{6}$/;
const BASES = /** @type {const} */ (["light", "dark"]);
// Every built-in theme by its ui.theme name, with the base its locked set comes from.
const THEME_NAMES = /** @type {const} */ (["light", "dark", "high_contrast"]);
const BASE_OF = /** @type {Object<string, "light" | "dark">} */ ({
  light: "light",
  dark: "dark",
  high_contrast: "dark",
});
/** @type {ThemeOptions} */
const DEFAULT_OPTIONS = { density: "comfortable", reducedMotion: false, colorblind: false };
/** @type {ThemeOptions[]} */
const EVERY_OPTION_SET = [
  DEFAULT_OPTIONS,
  { density: "compact", reducedMotion: false, colorblind: false },
  { density: "comfortable", reducedMotion: true, colorblind: false },
  { density: "compact", reducedMotion: true, colorblind: true },
];
// The locked tokens exactly as contracts 5.6 lists them.
const DOCUMENTED_LOCKED_VARIABLES = [
  "--rr-locked-severity-error",
  "--rr-locked-severity-warning",
  "--rr-locked-severity-info",
  "--rr-locked-egress",
  "--rr-locked-egress-text",
  "--rr-locked-security-surface",
  "--rr-locked-security-text",
  "--rr-locked-security-danger",
];
// The other variables contracts 5.6 documents, besides one --rr-color-<role> per role and --rr-space-<n> per step.
const DOCUMENTED_OTHER_VARIABLES = [
  "--rr-font-ui",
  "--rr-font-mono",
  "--rr-font-weight-regular",
  "--rr-font-weight-medium",
  "--rr-font-weight-bold",
  "--rr-font-size-xs",
  "--rr-font-size-sm",
  "--rr-font-size-md",
  "--rr-font-size-lg",
  "--rr-font-size-xl",
  "--rr-font-size-xxl",
  "--rr-line-height-body",
  "--rr-line-height-heading",
  "--rr-line-height-code",
  "--rr-radius-small",
  "--rr-radius-medium",
  "--rr-radius-large",
  "--rr-motion-fast",
  "--rr-motion-slow",
  "--rr-motion-easing",
  "--rr-focus-ring-width",
  "--rr-focus-ring-offset",
  "--rr-elevation-1",
  "--rr-elevation-2",
  "--rr-elevation-3",
];
const HOSTILE_COLOR = "#ff00ff";

/**
 * The color role keys of the generated schema.
 * @returns {string[]}
 */
function schemaColorRoles() {
  const schema = JSON.parse(readFileSync(THEME_SCHEMA_FILE, "utf8"));
  return Object.keys(schema.$defs.ThemeColors.properties).sort();
}

/**
 * A stand-in for document.documentElement that records every setProperty call in order.
 * @returns {{ style: { setProperty: (name: string, value: string) => void }, calls: string[][], values: Map<string, string> }}
 */
function recordingRoot() {
  /** @type {string[][]} */
  const calls = [];
  const values = new Map();
  return {
    calls,
    values,
    style: {
      setProperty: (name, value) => {
        calls.push([name, value]);
        values.set(name, value);
      },
    },
  };
}

/**
 * The blue channel of a #rrggbb color, 0 to 255: tells blue from orange in the data-scale test.
 * @param {string} hex
 * @returns {number}
 */
function blueChannel(hex) {
  return Number.parseInt(hex.slice(5, 7), 16);
}

/**
 * Tokens that try every way to name a locked token: as a role, as a CSS variable, at the top level.
 * @param {"light" | "dark"} base
 * @returns {ThemeTokens}
 */
function hostileTokens(base) {
  const tokens = builtInTheme(base, DEFAULT_OPTIONS);
  const colors = { ...tokens.colors };
  for (const variable of DOCUMENTED_LOCKED_VARIABLES) {
    const key = variable.replace("--rr-locked-", "");
    colors[variable] = HOSTILE_COLOR;
    colors[key] = HOSTILE_COLOR;
    colors[`locked_${key.replaceAll("-", "_")}`] = HOSTILE_COLOR;
  }
  return /** @type {ThemeTokens} */ ({
    ...tokens,
    colors,
    locked: { light: colors, dark: colors },
    [DOCUMENTED_LOCKED_VARIABLES[0]]: HOSTILE_COLOR,
  });
}

describe("the every_role fixture", () => {
  test("sets every color role of the schema and the xxl size, so it keeps up when a role is added", () => {
    const fixture = JSON.parse(readFileSync(EVERY_ROLE_FIXTURE, "utf8"));
    expect(Object.keys(fixture.colors).sort()).toEqual(schemaColorRoles());
    expect(fixture.typography.font_sizes_px.xxl).toBeDefined();
  });
});

describe("built-in themes", () => {
  test("the role keys are the generated schema's, and every built-in sets every one", () => {
    expect([...COLOR_ROLES].sort()).toEqual(schemaColorRoles());
    for (const name of THEME_NAMES) {
      expect(Object.keys(BUILT_IN_THEMES[name].colors).sort()).toEqual(schemaColorRoles());
      expect(BUILT_IN_THEMES[name].base).toBe(BASE_OF[name]);
    }
  });

  test("every role key resolves to a #rrggbb color in every built-in, under every option set", () => {
    for (const name of THEME_NAMES) {
      for (const options of EVERY_OPTION_SET) {
        const tokens = builtInTheme(name, options);
        for (const role of COLOR_ROLES) {
          expect(tokens.colors[role], `${name} ${role}`).toMatch(HEX_COLOR);
        }
        expect(tokens.base).toBe(BASE_OF[name]);
      }
    }
  });

  test("the light and dark canvas backgrounds are the design's values (ui-design.md 4.1)", () => {
    expect(BUILT_IN_THEMES.light.colors.canvas_background).toBe("#f3f4f6");
    expect(BUILT_IN_THEMES.dark.colors.canvas_background).toBe("#0d0f13");
  });

  test("the design's confidence tint is set in every built-in (ui-design.md 4.4)", () => {
    expect(BUILT_IN_THEMES.light.colors.confidence_tint).toBe("#c75300");
    expect(BUILT_IN_THEMES.dark.colors.confidence_tint).toBe("#f0883e");
    expect(BUILT_IN_THEMES.high_contrast.colors.confidence_tint).toBe("#ffb070");
  });

  test("the data scale is blue and orange by default: orange for negative, blue for positive", () => {
    for (const name of /** @type {const} */ (["light", "dark"])) {
      const colors = builtInTheme(name, DEFAULT_OPTIONS).colors;
      expect(blueChannel(colors.diverging_negative)).toBeLessThan(blueChannel(colors.diverging_positive));
      expect(colors.diverging_negative).not.toBe(colors.diverging_positive);
    }
    expect(BUILT_IN_THEMES.light.colors.diverging_negative).toBe("#c75300");
    expect(BUILT_IN_THEMES.light.colors.diverging_positive).toBe("#0a66d8");
  });

  test("the high-contrast built-in is its own theme with the design's values and the dark base", () => {
    const tokens = builtInTheme("high_contrast", DEFAULT_OPTIONS);
    expect(tokens.name).toBe("High contrast");
    expect(tokens.base).toBe("dark");
    expect(tokens.colors.background).toBe("#000000");
    expect(tokens.colors.text).toBe("#ffffff");
    expect(tokens.colors.focus_ring).toBe("#4de2ff");
    expect(tokens.focusRing.widthPx).toBe(3);
    expect(tokens.colors).not.toEqual(builtInTheme("dark", DEFAULT_OPTIONS).colors);
    expect(tokens.elevation.level1).toBe("0 0 0 1px #9a9a9a");
  });

  test("the high-contrast built-in takes the dark locked set", () => {
    const tokens = builtInTheme("high_contrast", DEFAULT_OPTIONS);
    expect(Object.fromEntries(lockedVariables(tokens))).toEqual(
      Object.fromEntries(lockedVariables(builtInTheme("dark", DEFAULT_OPTIONS))),
    );
  });

  test("a theme that leaves xxl out gets xl's size, and the built-ins set 28", () => {
    expect(BUILT_IN_THEMES.light.typography.font_sizes_px.xxl).toBe(28);
    expect(builtInTheme("light", DEFAULT_OPTIONS).typography.fontSizesPx.xxl).toBe(28);
    const light = BUILT_IN_THEMES.light;
    const { xxl, ...sizesWithoutXxl } = light.typography.font_sizes_px;
    const withoutXxl = /** @type {BuiltInTheme} */ ({
      ...light,
      typography: { ...light.typography, font_sizes_px: sizesWithoutXxl },
    });
    expect(resolveTheme(withoutXxl, DEFAULT_OPTIONS).typography.fontSizesPx.xxl).toBe(
      light.typography.font_sizes_px.xl,
    );
    expect(xxl).toBe(28);
  });

  test("density picks the spacing scale; both scales have the same steps", () => {
    for (const base of BASES) {
      const comfortable = builtInTheme(base, DEFAULT_OPTIONS).spacingPx;
      const compact = builtInTheme(base, { ...DEFAULT_OPTIONS, density: "compact" }).spacingPx;
      expect(comfortable).toEqual(BUILT_IN_THEMES[base].density.comfortable.spacing_px);
      expect(compact).toEqual(BUILT_IN_THEMES[base].density.compact.spacing_px);
      expect(compact.length).toBe(comfortable.length);
      expect(compact).not.toEqual(comfortable);
    }
  });

  test("reduced motion sets both durations to 0 and keeps the easing", () => {
    const tokens = builtInTheme("light", { ...DEFAULT_OPTIONS, reducedMotion: true });
    expect(tokens.motion).toEqual({ fastMs: 0, slowMs: 0, easing: BUILT_IN_THEMES.light.motion.easing });
    expect(builtInTheme("light", DEFAULT_OPTIONS).motion.fastMs).toBeGreaterThan(0);
  });

  test("the colorblind palette replaces only the series and diverging roles", () => {
    for (const base of BASES) {
      const plain = builtInTheme(base, DEFAULT_OPTIONS).colors;
      const colorblind = builtInTheme(base, { ...DEFAULT_OPTIONS, colorblind: true }).colors;
      const replaced = COLOR_ROLES.filter((role) => plain[role] !== colorblind[role]);
      expect(replaced.every((role) => role.startsWith("series_") || role.startsWith("diverging_"))).toBe(true);
      for (const [role, color] of Object.entries(COLORBLIND_COLORS[base])) {
        expect(COLOR_ROLES).toContain(role);
        expect(colorblind[role]).toBe(color);
      }
    }
  });

  test("an unknown base resolves to light rather than failing", () => {
    const tokens = builtInTheme(/** @type {"light"} */ ("sepia"), DEFAULT_OPTIONS);
    expect(tokens.base).toBe("light");
    expect(tokens.colors).toEqual(builtInTheme("light", DEFAULT_OPTIONS).colors);
  });
});

describe("applyThemeTokens", () => {
  test("sets every documented variable, for every built-in", () => {
    for (const name of THEME_NAMES) {
      const tokens = builtInTheme(name, DEFAULT_OPTIONS);
      const root = recordingRoot();
      applyThemeTokens(root, tokens);
      const expected = [
        ...COLOR_ROLES.map(colorVariableName),
        ...tokens.spacingPx.map((_, step) => `--rr-space-${step}`),
        ...DOCUMENTED_OTHER_VARIABLES,
        ...DOCUMENTED_LOCKED_VARIABLES,
      ];
      for (const name of expected) {
        expect(root.values.get(name), name).toBeTruthy();
      }
      const written = [...root.values.keys()].filter((name) => name.startsWith("--"));
      expect(written.sort()).toEqual([...expected].sort());
    }
  });

  test("writes the role colors under their hyphenated names", () => {
    const root = recordingRoot();
    applyThemeTokens(root, builtInTheme("dark", DEFAULT_OPTIONS));
    expect(root.values.get("--rr-color-surface-raised")).toBe(BUILT_IN_THEMES.dark.colors.surface_raised);
    expect(root.values.get("--rr-color-canvas-background")).toBe("#0d0f13");
    expect(root.values.get("--rr-font-size-xxl")).toBe("28px");
    expect(root.values.get("--rr-space-2")).toBe(`${BUILT_IN_THEMES.dark.density.comfortable.spacing_px[2]}px`);
    expect(root.values.get("color-scheme")).toBe("dark");
  });

  test("writes the locked variables last, from LOCKED_TOKENS for the base", () => {
    for (const base of BASES) {
      const root = recordingRoot();
      applyThemeTokens(root, builtInTheme(base, DEFAULT_OPTIONS));
      const lastNames = root.calls.slice(-DOCUMENTED_LOCKED_VARIABLES.length).map(([name]) => name);
      expect(lastNames.sort()).toEqual([...DOCUMENTED_LOCKED_VARIABLES].sort());
      for (const variable of DOCUMENTED_LOCKED_VARIABLES) {
        expect(root.values.get(variable)).toBe(LOCKED_TOKENS[base][variable.replace("--rr-locked-", "")]);
      }
    }
  });

  test("a theme object carrying locked names leaves the locked variables unchanged", () => {
    for (const base of BASES) {
      const root = recordingRoot();
      applyThemeTokens(root, builtInTheme(base, DEFAULT_OPTIONS));
      const before = DOCUMENTED_LOCKED_VARIABLES.map((name) => root.values.get(name));
      applyThemeTokens(root, hostileTokens(base));
      const after = DOCUMENTED_LOCKED_VARIABLES.map((name) => root.values.get(name));
      expect(after).toEqual(before);
      expect(after).not.toContain(HOSTILE_COLOR);
      // The stray keys never become variables of their own either.
      expect([...root.values.keys()].filter((name) => root.values.get(name) === HOSTILE_COLOR)).toEqual([]);
    }
  });

  test("a theme that sets every role to one color still gets the locked set of its base", () => {
    const tokens = builtInTheme("dark", DEFAULT_OPTIONS);
    const oneColor = Object.fromEntries(COLOR_ROLES.map((role) => [role, HOSTILE_COLOR]));
    const variables = lockedVariables({ ...tokens, colors: oneColor });
    expect(Object.fromEntries(variables)).toEqual(Object.fromEntries(lockedVariables(tokens)));
  });

  test("themeVariables never writes a locked variable", () => {
    const names = themeVariables(hostileTokens("light")).map(([name]) => name);
    expect(names.filter((name) => name.startsWith("--rr-locked-"))).toEqual([]);
  });

  test("both locked sets hold exactly the documented tokens", () => {
    for (const base of BASES) {
      const names = Object.keys(LOCKED_TOKENS[base]).map((key) => `--rr-locked-${key}`);
      expect(names.sort()).toEqual([...DOCUMENTED_LOCKED_VARIABLES].sort());
      for (const color of Object.values(LOCKED_TOKENS[base])) {
        expect(color).toMatch(HEX_COLOR);
      }
    }
  });
});

describe("renderer colors", () => {
  test("canvasColors gives RGBA arrays from 0 to 1 and the label font", () => {
    const colors = canvasColors(builtInTheme("dark", DEFAULT_OPTIONS));
    expect(colors.background).toEqual([13 / 255, 15 / 255, 19 / 255, 1]);
    const roles = [colors.grid, colors.node, colors.nodeBorder, colors.nodeSelected, colors.edge, colors.edgeActive];
    for (const rgba of [...roles, colors.label, colors.port]) {
      expect(rgba).toHaveLength(4);
      expect(rgba.every((channel) => channel >= 0 && channel <= 1)).toBe(true);
    }
    expect(colors.labelFont).toEqual({
      family: BUILT_IN_THEMES.dark.typography.font_family_ui,
      weight: BUILT_IN_THEMES.dark.typography.font_weights.regular,
      sizePx: BUILT_IN_THEMES.dark.typography.font_sizes_px.sm,
    });
  });

  test("vizColors follows the colorblind setting", () => {
    const plain = vizColors(builtInTheme("light", DEFAULT_OPTIONS));
    const colorblind = vizColors(builtInTheme("light", { ...DEFAULT_OPTIONS, colorblind: true }));
    expect(plain.series).toHaveLength(8);
    expect(plain.diverging).toHaveLength(3);
    expect(plain.sequential).toHaveLength(2);
    expect(colorblind.series).not.toEqual(plain.series);
    expect(colorblind.axis).toEqual(plain.axis);
  });

  test("a malformed color throws instead of drawing a guess", () => {
    const tokens = builtInTheme("light", DEFAULT_OPTIONS);
    expect(() => canvasColors({ ...tokens, colors: { ...tokens.colors, canvas_edge: "red" } })).toThrow(/rrggbb/);
  });
});
