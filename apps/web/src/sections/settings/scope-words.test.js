// The presets the Access choices show and the words say what each can do. The daemon's own presets are checked against
// these in tests/e2e/token-presets.spec.js.

import { describe, expect, test } from "vitest";

import { PRESET_LABELS, PRESET_SCOPES, presetWords } from "./scope-words.js";

describe("the presets", () => {
  test("every preset has a label, and every label is a preset", () => {
    expect(Object.keys(PRESET_LABELS).sort()).toEqual(Object.keys(PRESET_SCOPES).sort());
  });

  test("chat-only can chat and cannot read or change projects", () => {
    const words = presetWords("chat-only");
    expect(words.canDo).toEqual(["Chat with local models and use the /v1 endpoints"]);
    expect(words.cannotDo).toContain("Change settings and tokens, collect garbage, and update");
  });

  test("a preset's words never list a scope it grants as one it withholds", () => {
    for (const preset of Object.keys(PRESET_SCOPES)) {
      const words = presetWords(preset);
      for (const word of words.canDo) {
        expect(words.cannotDo).not.toContain(word);
      }
    }
  });
});
