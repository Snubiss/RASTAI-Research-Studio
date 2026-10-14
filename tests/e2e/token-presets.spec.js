// The scope presets the Tokens page words (apps/web/src/sections/settings/scope-words.js) against the real daemon (layout
// frontend-and-plugins.md 8.7; plan M1-54). The words say what a token can and cannot do, so they must match the scopes
// the daemon gives each preset, and the Access choices must be the presets the daemon takes. A change on either side
// fails here, not in a security prompt that misstates a token.

import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { PRESET_LABELS, PRESET_SCOPES } from "../../apps/web/src/sections/settings/scope-words.js";
import { goToSection, jsonLines, openApp, startDaemon } from "./harness.js";

// `token list --json` answers with one line that has a `tokens` list; the lines after it are the exit record.
const LIST_KEY = "tokens";
// The daemon's own preset names come from the token.create help, which prints the argument's enum (contracts 6.3).
const PRESET_FLAG_PATTERN = /--preset <one of ([^>]+)>/;

/**
 * The presets the daemon takes, as token.create's --preset enum names them.
 * @param {{ cli: (args: string[]) => Promise<string> }} daemon
 * @returns {Promise<string[]>}
 */
async function daemonPresets(daemon) {
  const help = await daemon.cli(["token", "create", "--help"]);
  const match = PRESET_FLAG_PATTERN.exec(help);
  assert.ok(match, `token create's help names its presets: ${help}`);
  return match[1].split(",").map((preset) => preset.trim());
}

describe("the presets' scopes", () => {
  test("the words know every preset the daemon takes, and each token holds the scopes they say", async () => {
    const daemon = await startDaemon();
    try {
      const presets = await daemonPresets(daemon);
      assert.deepEqual(
        Object.keys(PRESET_SCOPES).sort(),
        [...presets].sort(),
        "PRESET_SCOPES has the daemon's presets",
      );
      for (const preset of presets) {
        const name = `preset ${preset}`;
        await daemon.cli(["token", "create", name, "--preset", preset, "--json"]);
      }
      const listed = jsonLines(await daemon.cli(["token", "list", "--json"])).find((line) => LIST_KEY in line);
      for (const preset of presets) {
        const token = listed.tokens.find((row) => row.name === `preset ${preset}`);
        assert.ok(token, `token.list has the ${preset} token`);
        assert.deepEqual([...token.scopes].sort(), [...PRESET_SCOPES[preset]].sort(), `the ${preset} preset's scopes`);
      }
    } finally {
      await daemon.stop();
    }
  });
});

describe("the Access choices", () => {
  test("the Tokens page offers exactly the presets the words know, with their labels", async () => {
    const daemon = await startDaemon();
    const app = await openApp("chromium", daemon);
    try {
      const presets = await daemonPresets(daemon);
      await goToSection(app.page, "Settings");
      const options = await app.page
        .getByLabel("Access", { exact: true })
        .evaluate((select) =>
          [...select.options].map((option) => ({ value: option.value, label: option.textContent ?? "" })),
        );
      assert.deepEqual(
        options.map((option) => option.value).sort(),
        [...presets].sort(),
        "the Access values are the daemon's presets",
      );
      for (const option of options) {
        assert.equal(option.label, PRESET_LABELS[option.value], `the label of ${option.value}`);
      }
    } finally {
      await app.close();
      await daemon.stop();
    }
  });
});
