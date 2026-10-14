// Shortcut text and key presses: Mod is Ctrl or Cmd, Shift and Alt match exactly, and the copy-as shortcut is Mod+.

import { describe, expect, test } from "vitest";

import { COPY_AS_CLI_SHORTCUT, commandForKeys, parseShortcut, shortcutLabel, shortcutMatches } from "./keybindings.js";

/** @param {Partial<import("./keybindings.js").KeyPress>} fields */
function press(fields) {
  return { key: "", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...fields };
}

describe("parseShortcut", () => {
  test("reads the modifiers and the key, and reads empty text as no shortcut", () => {
    expect(parseShortcut("Mod+Shift+L")).toEqual({ mod: true, shift: true, alt: false, key: "l" });
    expect(parseShortcut("")).toBeNull();
    expect(parseShortcut("Hyper+L")).toBeNull();
  });
});

describe("shortcutMatches", () => {
  test("Mod is Ctrl or Cmd, and Shift must match exactly", () => {
    expect(shortcutMatches("Mod+Shift+L", press({ key: "L", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(shortcutMatches("Mod+Shift+L", press({ key: "l", metaKey: true, shiftKey: true }))).toBe(true);
    expect(shortcutMatches("Mod+Shift+L", press({ key: "l", ctrlKey: true }))).toBe(false);
    expect(shortcutMatches("Mod+K", press({ key: "k", ctrlKey: true, shiftKey: true }))).toBe(false);
  });

  test("the copy-as shortcut is Ctrl period, and not Ctrl Shift C, which opens the browser's element picker", () => {
    expect(COPY_AS_CLI_SHORTCUT).toBe("Mod+.");
    expect(shortcutMatches(COPY_AS_CLI_SHORTCUT, press({ key: ".", ctrlKey: true }))).toBe(true);
    expect(shortcutMatches(COPY_AS_CLI_SHORTCUT, press({ key: "c", ctrlKey: true, shiftKey: true }))).toBe(false);
  });
});

describe("shortcutLabel", () => {
  test("writes Mod as Ctrl and capitalises the key", () => {
    expect(shortcutLabel("Mod+Shift+L")).toBe("Ctrl+Shift+L");
    expect(shortcutLabel("Mod+.")).toBe("Ctrl+.");
    expect(shortcutLabel("")).toBe("");
  });
});

describe("commandForKeys", () => {
  test("finds the command whose default shortcut the key press is, and none for a command without one", () => {
    const commands = [
      { commandId: "system.status", defaultShortcut: "" },
      { commandId: "job.cancel", defaultShortcut: "Mod+Shift+L" },
    ];
    expect(commandForKeys(commands, press({ key: "l", ctrlKey: true, shiftKey: true }))?.commandId).toBe("job.cancel");
    expect(commandForKeys(commands, press({ key: "x", ctrlKey: true }))).toBeNull();
  });
});
