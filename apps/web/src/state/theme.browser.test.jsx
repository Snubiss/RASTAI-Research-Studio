// `system` follows the OS with no command, end to end (layout frontend-and-plugins.md 2.10, 8.4; design 4): a shell
// started with ui.theme=system and a change of the OS color scheme or reduced-motion preference re-resolves the display
// and rewrites the --rr-* variables on the page, and the daemon is asked for no settings command. The pure functions
// are tested in theme.test.js; this test covers the wiring in the shell's start.

import { afterEach, describe, expect, test } from "vitest";

import { testShell } from "../testing/fake-shell.js";
import { inAct } from "../testing/render.js";

const BACKGROUND_VARIABLE = "--rr-color-background";

/** Puts the stub back after each test. */
let restoreMatchMedia = () => {};

afterEach(() => {
  restoreMatchMedia();
  restoreMatchMedia = () => {};
});

/**
 * A MediaQueryList the test changes by hand.
 * @typedef {object} FakeMediaQuery
 * @property {boolean} matches
 * @property {(type: string, listener: () => void) => void} addEventListener
 * @property {(type: string, listener: () => void) => void} removeEventListener
 * @property {(next: boolean) => void} set   changes the value and tells each listener, as the browser does
 */

/**
 * @param {boolean} matches
 * @returns {FakeMediaQuery}
 */
function fakeMediaQuery(matches) {
  /** @type {Set<() => void>} */
  const listeners = new Set();
  return {
    matches,
    /** @param {string} _type @param {() => void} listener */
    addEventListener: (_type, listener) => listeners.add(listener),
    /** @param {string} _type @param {() => void} listener */
    removeEventListener: (_type, listener) => listeners.delete(listener),
    /** @param {boolean} next */
    set(next) {
      this.matches = next;
      listeners.forEach((listener) => listener());
    },
  };
}

/**
 * Makes window.matchMedia answer with the two fake queries, and restores the browser's after the test.
 * @param {{ dark: FakeMediaQuery, reduced: FakeMediaQuery }} queries
 */
function stubMatchMedia(queries) {
  const original = window.matchMedia;
  // The fake answers only the two queries the shell asks; the cast is the test's, since it is not a full MediaQueryList.
  window.matchMedia = /** @type {any} */ (
    (/** @type {string} */ query) => (query.includes("prefers-color-scheme") ? queries.dark : queries.reduced)
  );
  restoreMatchMedia = () => {
    window.matchMedia = original;
  };
}

/**
 * The settings commands the client was asked for.
 * @param {{ calls: { commandId: string }[] }} client
 * @returns {{ commandId: string }[]}
 */
function settingsCalls(client) {
  return client.calls.filter((call) => call.commandId.startsWith("settings."));
}

/** @param {string} name */
function pageVariable(name) {
  return document.documentElement.style.getPropertyValue(name);
}

describe("system follows the OS", () => {
  test("an OS color-scheme change re-resolves the theme and the page variables, with no settings command", async () => {
    const queries = { dark: fakeMediaQuery(false), reduced: fakeMediaQuery(false) };
    stubMatchMedia(queries);
    const { shell, client } = await testShell({ settings: { ui: { theme: "system" } } });
    expect(shell.stores.display.get().theme.base).toBe("light");
    const lightBackground = pageVariable(BACKGROUND_VARIABLE);

    await inAct(() => queries.dark.set(true));

    expect(shell.stores.display.get().theme.base).toBe("dark");
    expect(pageVariable(BACKGROUND_VARIABLE)).not.toBe(lightBackground);
    expect(settingsCalls(client)).toEqual([]);
  });

  test("an OS reduced-motion change re-resolves the motion tokens, with no settings command", async () => {
    const queries = { dark: fakeMediaQuery(false), reduced: fakeMediaQuery(false) };
    stubMatchMedia(queries);
    const { shell, client } = await testShell({ settings: { ui: { theme: "system" } } });
    expect(shell.stores.display.get().theme.motion.fastMs).not.toBe(0);

    await inAct(() => queries.reduced.set(true));

    expect(shell.stores.display.get().theme.motion.fastMs).toBe(0);
    expect(settingsCalls(client)).toEqual([]);
  });
});
