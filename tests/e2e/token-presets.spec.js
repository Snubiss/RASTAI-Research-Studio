// The scope presets the Tokens page words (apps/web/src/sections/settings/scope-words.js) against the real daemon (layout
// frontend-and-plugins.md 8.7; plan M1-54). The words say what a token can and cannot do, so they must match the scopes
// the daemon gives each preset, and the Access choices must be the presets the daemon takes. A change on either side
// fails here, not in a security prompt that misstates a token.

import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { PRESET_LABELS, PRESET_SCOPES } from "../../apps/web/src/sections/settings/scope-words.js";
import { ENGINES, goToSection, jsonLines, openApp, startDaemon } from "./harness.js";

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

// The Tokens page end to end (ui-design 6.9; plan M1-54): the locked prompt makes the token only when it is approved, the
// value shows once, and the list holds the token while the value is gone from the page.
const E2E_TOKEN_NAME = "e2e token";
// A preset the daemon doesn't take: the page sends it as chosen, and the daemon refuses it with a catalog code.
const REFUSED_PRESET = "no-such-preset";
// The code the daemon gives an unknown preset (crates/rastai-formats/src/commands/mod.rs). A diagnostic's toast ends
// with its catalog code in parentheses (apps/web/src/app/shell.js notify).
const REFUSED_PRESET_TOAST_CODE = "(RRS-E1004)";

for (const engine of ENGINES) {
  describe(`the Tokens page in ${engine}`, () => {
    test("create, approve, show once, Done, list, then revoke", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await app.page.getByLabel("Token name", { exact: true }).fill(E2E_TOKEN_NAME);
        await app.page.getByRole("button", { name: "Create token", exact: true }).click();
        const prompt = app.page.getByRole("dialog");
        await prompt.waitFor();
        await prompt.getByRole("button", { name: "Create token", exact: true }).click();
        const shown = await prompt.locator("code").innerText();
        assert.ok(shown.length > 0, "the prompt shows the new token's value");
        await prompt.getByRole("button", { name: "Done", exact: true }).click();
        await prompt.waitFor({ state: "detached" });
        const row = app.page.locator(".rr-token-list__item", { hasText: E2E_TOKEN_NAME });
        await row.waitFor();
        assert.ok(
          !(await app.page.content()).includes(shown),
          "the token's value is not on the page once Done is pressed",
        );
        await app.page.getByRole("button", { name: `Revoke ${E2E_TOKEN_NAME}`, exact: true }).click();
        await row.waitFor({ state: "detached" });
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("a refused create closes the prompt and shows the daemon's diagnostic with its code", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await app.page.getByLabel("Token name", { exact: true }).fill(E2E_TOKEN_NAME);
        // The page's Access list takes only the presets it offers, so the refused one is added to it as a chosen option.
        await app.page.getByLabel("Access", { exact: true }).evaluate((select, preset) => {
          const option = document.createElement("option");
          option.value = preset;
          option.textContent = preset;
          select.append(option);
          select.value = preset;
          select.dispatchEvent(new Event("change", { bubbles: true }));
        }, REFUSED_PRESET);
        await app.page.getByRole("button", { name: "Create token", exact: true }).click();
        const prompt = app.page.getByRole("dialog");
        await prompt.waitFor();
        await prompt.getByRole("button", { name: "Create token", exact: true }).click();
        await prompt.waitFor({ state: "detached" });
        await app.page.locator(".rr-toast").first().waitFor();
        const toasts = await app.page.locator(".rr-toast").allInnerTexts();
        assert.ok(
          toasts.some((text) => text.includes(REFUSED_PRESET_TOAST_CODE)),
          `a toast names the catalog code ${REFUSED_PRESET_TOAST_CODE}: ${toasts.join(" | ")}`,
        );
        assert.equal(await app.page.locator(".rr-token-list__item", { hasText: E2E_TOKEN_NAME }).count(), 0);
      } finally {
        await app.close();
        await daemon.stop();
      }
    });
  });
}
