// Themes and density in the built app (layout frontend-and-plugins.md 2.10, 8.7; ui-design 6.8; plan M1-54): the theme cards
// switch the shell at once, `system` follows the OS scheme with no command, compact density restyles the shell, and no
// locked security variable changes under any of them.

import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { themeVariables } from "../../packages/ui/src/tokens/apply-theme.js";
import { builtInTheme } from "../../packages/ui/src/tokens/resolve-tokens.js";
import { LOCKED_PREFIX, LOCKED_TOKENS } from "../../packages/ui/src/tokens/locked.js";
import { ENGINES, goToSection, openApp, startDaemon } from "./harness.js";

const BACKGROUND_VARIABLE = "--rr-color-background";
const SPACING_VARIABLE = "--rr-space-3";
// The options every built-in theme is resolved with here: the page's default density and motion (ui-design 6.8).
const RESOLVE_OPTIONS = { density: "comfortable", reducedMotion: false, colorblind: false };

for (const engine of ENGINES) {
  describe(`themes in ${engine}`, () => {
    test("a theme card switches the page at once, without a reload", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await app.page.evaluate(() => {
          window.__noReloadMarker = true;
        });
        const before = await cssVariable(app.page, BACKGROUND_VARIABLE);
        await themeRadio(app.page, "dark").check();
        await app.page.waitForFunction(
          ([name, previous]) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() !== previous,
          [BACKGROUND_VARIABLE, before],
        );
        assert.equal(await app.page.evaluate(() => window.__noReloadMarker), true, "the page was not reloaded");
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("the locked security colors follow the base scheme alone, so every theme on one base shows the same ones", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon, { colorScheme: "light" });
      try {
        await goToSection(app.page, "Settings");
        // The light base has one locked set and the dark base another. Light, and system with a light OS, show the
        // light set; dark, high contrast (dark base) and system with a dark OS show the dark set. No choice changes a set.
        // Each choice also checks the shell's own background, so a theme that never applied can't pass on the locked set.
        await themeRadio(app.page, "light").check();
        await waitForLockedBase(app.page, "light");
        assert.deepEqual(await lockedVariables(app.page), lockedFor("light"));
        await assertShellTheme(app.page, "light", "light");
        for (const value of ["dark", "high_contrast"]) {
          await themeRadio(app.page, value).check();
          await waitForLockedBase(app.page, "dark");
          assert.deepEqual(await lockedVariables(app.page), lockedFor("dark"), `${value} shows the dark locked set`);
          await assertShellTheme(app.page, value, "dark");
        }
        await themeRadio(app.page, "system").check();
        await app.page.emulateMedia({ colorScheme: "dark" });
        await waitForLockedBase(app.page, "dark");
        assert.deepEqual(await lockedVariables(app.page), lockedFor("dark"), "system with a dark OS");
        await assertShellTheme(app.page, "system", "dark");
        await app.page.emulateMedia({ colorScheme: "light" });
        await waitForLockedBase(app.page, "light");
        assert.deepEqual(await lockedVariables(app.page), lockedFor("light"), "system with a light OS");
        await assertShellTheme(app.page, "system", "light");
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("compact density restyles the shell", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        const comfortable = await cssVariable(app.page, SPACING_VARIABLE);
        const lockedBefore = await lockedVariables(app.page);
        await app.page.locator('input[name="density"][value="compact"]').check();
        await app.page.waitForFunction(
          ([name, previous]) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() !== previous,
          [SPACING_VARIABLE, comfortable],
        );
        assert.deepEqual(await lockedVariables(app.page), lockedBefore, "density leaves the locked colors alone");
      } finally {
        await app.close();
        await daemon.stop();
      }
    });
  });
}

/**
 * Waits until the shell shows the theme's own colors, then checks the background variable is that theme's. The theme is
 * named as the card's value; a system choice is shown as the OS scheme it follows.
 * @param {import("playwright").Page} page
 * @param {string} value   a ui.theme value: system, light, dark or high_contrast
 * @param {"light" | "dark"} scheme   the scheme the shell should show (for system, the OS's)
 */
async function assertShellTheme(page, value, scheme) {
  const themeName = value === "system" ? scheme : value;
  const expected = backgroundOf(themeName);
  await page.waitForFunction(
    ([variable, wanted]) => getComputedStyle(document.documentElement).getPropertyValue(variable).trim() === wanted,
    [BACKGROUND_VARIABLE, expected],
  );
  assert.equal(await cssVariable(page, BACKGROUND_VARIABLE), expected, `${value} shows its own background`);
}

/**
 * The background variable a built-in theme writes, as the page shows it.
 * @param {string} themeName   light, dark or high_contrast
 * @returns {string}
 */
function backgroundOf(themeName) {
  const variables = themeVariables(builtInTheme(themeName, RESOLVE_OPTIONS));
  const found = variables.find(([name]) => name === BACKGROUND_VARIABLE);
  assert.ok(found, `${themeName} has a background variable`);
  return found[1].trim();
}

/**
 * The radio of one theme card, by its ui.theme value.
 * @param {import("playwright").Page} page
 * @param {string} value
 * @returns {import("playwright").Locator}
 */
function themeRadio(page, value) {
  return page.locator(`input[name="theme"][value="${value}"]`);
}

/**
 * @param {import("playwright").Page} page
 * @param {string} name   a custom property, such as --rr-color-background
 * @returns {Promise<string>}
 */
async function cssVariable(page, name) {
  return page.evaluate(
    (variable) => getComputedStyle(document.documentElement).getPropertyValue(variable).trim(),
    name,
  );
}

/**
 * Waits until the page shows a base's locked set: the security text is the base's first token.
 * @param {import("playwright").Page} page
 * @param {"light" | "dark"} base
 */
async function waitForLockedBase(page, base) {
  await page.waitForFunction(
    ([variable, expected]) => getComputedStyle(document.documentElement).getPropertyValue(variable).trim() === expected,
    [`${LOCKED_PREFIX}security-text`, LOCKED_TOKENS[base]["security-text"]],
  );
}

/**
 * The locked variables a base has, as the page should show them (the names lockedVariables reads), sorted by name.
 * @param {"light" | "dark"} base
 * @returns {Object<string, string>}
 */
function lockedFor(base) {
  const entries = Object.entries(LOCKED_TOKENS[base]).map(([key, value]) => [`${LOCKED_PREFIX}${key}`, value]);
  return Object.fromEntries(entries.sort(([first], [second]) => (first < second ? -1 : 1)));
}

/**
 * Every --rr-locked-* variable on the page and its value, sorted by name.
 * @param {import("playwright").Page} page
 * @returns {Promise<Object<string, string>>}
 */
async function lockedVariables(page) {
  return page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    const names = [...document.documentElement.style].filter((name) => name.startsWith("--rr-locked-")).sort();
    return Object.fromEntries(names.map((name) => [name, style.getPropertyValue(name).trim()]));
  });
}
