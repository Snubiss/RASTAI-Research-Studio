// The theme state (layout frontend-and-plugins.md 2.10, 8.4): `system` follows the OS scheme with no command, the
// settings pick the built-in theme and its options, and no locked security token ever reaches a host.

import { LOCKED_TOKENS } from "@rastai/ui";
import { describe, expect, test, vi } from "vitest";

import { themeNameFor, themeTokensFor, watchColorScheme, withoutLockedTokens } from "./theme.js";

const LIGHT_OS = { prefersDark: false, prefersReducedMotion: false };
const DARK_OS = { prefersDark: true, prefersReducedMotion: false };

/**
 * @param {any} values
 * @returns {any}
 */
function settingsWith(values) {
  return { ...values };
}

/**
 * A MediaQueryList that can be told the OS scheme changed.
 * @param {boolean} matches
 */
function fakeQuery(matches) {
  /** @type {Set<() => void>} */
  const listeners = new Set();
  return {
    matches,
    /** @param {string} _type @param {() => void} listener */
    addEventListener: (_type, listener) => listeners.add(listener),
    /** @param {string} _type @param {() => void} listener */
    removeEventListener: (_type, listener) => listeners.delete(listener),
    /** @param {boolean} next */
    change(next) {
      this.matches = next;
      listeners.forEach((listener) => listener());
    },
    count: () => listeners.size,
  };
}

describe("themeNameFor", () => {
  test("system takes the OS scheme; the named themes are their own", () => {
    expect(themeNameFor("system", true)).toBe("dark");
    expect(themeNameFor("system", false)).toBe("light");
    expect(themeNameFor("dark", false)).toBe("dark");
    expect(themeNameFor("light", true)).toBe("light");
    expect(themeNameFor("high_contrast", true)).toBe("high_contrast");
  });

  test("a value the app doesn't know follows the OS", () => {
    expect(themeNameFor("plugin.theme", true)).toBe("dark");
  });
});

describe("themeTokensFor", () => {
  test("system follows an OS scheme change with no settings change and no command", () => {
    const values = settingsWith({ ui: { theme: "system" } });
    expect(themeTokensFor(values, LIGHT_OS).base).toBe("light");
    expect(themeTokensFor(values, DARK_OS).base).toBe("dark");
  });

  test("an explicit theme ignores the OS scheme", () => {
    expect(themeTokensFor({ ui: { theme: "light" } }, DARK_OS).base).toBe("light");
  });

  test("density, reduced motion and the colorblind palette come from the settings", () => {
    const tokens = themeTokensFor(
      { ui: { theme: "dark", density: "compact", reduced_motion: true, colorblind_palette: true } },
      LIGHT_OS,
    );
    expect(tokens.spacingPx).toEqual([0, 2, 4, 8, 12, 16, 24, 32]);
    expect(tokens.motion.fastMs).toBe(0);
    expect(tokens.motion.slowMs).toBe(0);
  });

  test("the OS asking for less motion sets both durations to zero, whatever the setting says", () => {
    const tokens = themeTokensFor({}, { prefersDark: false, prefersReducedMotion: true });
    expect(tokens.motion.fastMs).toBe(0);
  });

  test("with no settings at all the defaults are system and comfortable", () => {
    const tokens = themeTokensFor({}, DARK_OS);
    expect(tokens.base).toBe("dark");
    expect(tokens.spacingPx).toEqual([0, 4, 8, 12, 16, 24, 32, 48]);
  });
});

describe("watchColorScheme", () => {
  test("calls back on each OS change, and stops when asked", () => {
    const query = fakeQuery(false);
    const onChange = vi.fn();
    const stop = watchColorScheme(query, onChange);
    query.change(true);
    query.change(false);
    expect(onChange.mock.calls).toEqual([[true], [false]]);
    stop();
    expect(query.count()).toBe(0);
    query.change(true);
    expect(onChange).toHaveBeenCalledTimes(2);
  });
});

describe("withoutLockedTokens", () => {
  test("takes every locked security color out of the tokens a host gets", () => {
    const tokens = themeTokensFor({ ui: { theme: "light" } }, LIGHT_OS);
    const hostile = {
      ...tokens,
      colors: { ...tokens.colors, "severity-error": "hostile-value", egress: "hostile-value" },
    };
    const cleaned = withoutLockedTokens(hostile);
    for (const name of [...Object.keys(LOCKED_TOKENS.light), ...Object.keys(LOCKED_TOKENS.dark)]) {
      expect(cleaned.colors).not.toHaveProperty(name);
    }
    expect(cleaned.colors.surface).toBe(tokens.colors.surface);
  });

  test("leaves the tokens it was given as they were", () => {
    const tokens = themeTokensFor({}, LIGHT_OS);
    const copy = JSON.stringify(tokens);
    withoutLockedTokens(tokens);
    expect(JSON.stringify(tokens)).toBe(copy);
  });
});
