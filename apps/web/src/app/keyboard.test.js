// The global keys (design 3.7): Ctrl K from anywhere, Shift ? outside text fields, "G then a letter" to a section, and F6.
// keyStep is pure, so every row of the table is checked here; browser tests check the same keys in the real page.

import { describe, expect, test } from "vitest";

import {
  ACTION_FOCUS_REGION,
  ACTION_GO_PREFIX,
  ACTION_NONE,
  ACTION_PALETTE,
  ACTION_SHORTCUTS,
  keyStep,
} from "./keyboard.js";

/**
 * @param {Partial<{ key: string, ctrlOrMeta: boolean, alt: boolean, inTextField: boolean }>} parts
 */
function press(parts) {
  return { key: "", ctrlOrMeta: false, alt: false, inTextField: false, ...parts };
}

describe("keyStep", () => {
  test("Ctrl K and Cmd K open the palette, even inside a text field", () => {
    expect(keyStep(press({ key: "k", ctrlOrMeta: true }), false).action).toBe(ACTION_PALETTE);
    expect(keyStep(press({ key: "K", ctrlOrMeta: true, inTextField: true }), false).action).toBe(ACTION_PALETTE);
  });

  test("Shift ? shows the shortcut list outside text fields only", () => {
    expect(keyStep(press({ key: "?" }), false).action).toBe(ACTION_SHORTCUTS);
    expect(keyStep(press({ key: "?", inTextField: true }), false).action).toBe(ACTION_NONE);
  });

  test("G arms the prefix, and the next letter goes to its section", () => {
    const armed = keyStep(press({ key: "g" }), false);
    expect(armed).toEqual({ action: ACTION_NONE, goPending: true });
    expect(keyStep(press({ key: "h" }), armed.goPending).action).toBe(`${ACTION_GO_PREFIX}home`);
    expect(keyStep(press({ key: "C" }), true).action).toBe(`${ACTION_GO_PREFIX}inference`);
    expect(keyStep(press({ key: "t" }), true).action).toBe(`${ACTION_GO_PREFIX}training`);
    expect(keyStep(press({ key: "d" }), true).action).toBe(`${ACTION_GO_PREFIX}designer`);
    expect(keyStep(press({ key: "s" }), true).action).toBe(`${ACTION_GO_PREFIX}settings`);
    expect(keyStep(press({ key: "p" }), true).action).toBe(`${ACTION_GO_PREFIX}psychology`);
  });

  test("a letter that isn't a G target ends the prefix without acting", () => {
    expect(keyStep(press({ key: "x" }), true)).toEqual({ action: ACTION_NONE, goPending: false });
  });

  test("the letters do nothing while typing, and G doesn't arm there", () => {
    expect(keyStep(press({ key: "g", inTextField: true }), false)).toEqual({ action: ACTION_NONE, goPending: false });
    expect(keyStep(press({ key: "h", inTextField: true }), true).action).toBe(ACTION_NONE);
  });

  test("a chord with Ctrl or Alt is never a single-letter shortcut", () => {
    expect(keyStep(press({ key: "g", ctrlOrMeta: true }), false).goPending).toBe(false);
    expect(keyStep(press({ key: "g", alt: true }), false).goPending).toBe(false);
  });

  test("F6 moves focus between the regions, even from inside a field", () => {
    expect(keyStep(press({ key: "F6" }), false).action).toBe(ACTION_FOCUS_REGION);
    expect(keyStep(press({ key: "F6", inTextField: true }), false).action).toBe(ACTION_FOCUS_REGION);
  });
});
