// The settings a window shows before the daemon answers: values read by dotted key, and a change shown at once.

import { describe, expect, test } from "vitest";

import { readSetting, withSetting } from "./settings.js";

const STORED = { values: { ui: { theme: "system", density: "comfortable" } }, sources: [] };

describe("withSetting", () => {
  test("replaces one key and keeps the rest of the result", () => {
    const next = withSetting(STORED, "ui.density", "compact");
    expect(readSetting(next.values, "ui.density", "")).toBe("compact");
    expect(readSetting(next.values, "ui.theme", "")).toBe("system");
    expect(readSetting(STORED.values, "ui.density", "")).toBe("comfortable");
  });

  test("a key in a section the stored settings don't have yet is added", () => {
    const next = withSetting({ values: {}, sources: [] }, "ui.theme", "dark");
    expect(next.values).toEqual({ ui: { theme: "dark" } });
  });
});

describe("the dotted key", () => {
  test("a key with more than two parts is refused, not written under a wrong name", () => {
    expect(() => withSetting(STORED, "ui.theme.extra", "dark")).toThrow(/"section.name"/);
    expect(() => readSetting(STORED.values, "ui", "")).toThrow(/"section.name"/);
  });
});
