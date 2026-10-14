// Basic and Advanced in the Settings, Plugins and Remote sections (layout frontend-and-plugins.md 2.4, 8.7; plan M1-54):
// what Basic shows is a subsequence of what Advanced shows, a value typed in Basic survives a switch to Advanced and back,
// and a panel's own mode wins over the global one.

import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { ENGINES, goToSection, interactiveElements, openApp, setMode, startDaemon } from "./harness.js";

const SECTIONS = ["Settings", "Plugins", "Remote"];
const TOKEN_NAME_LABEL = "Token name";
const TYPED_NAME = "laptop";
const PANEL_MODE_LABEL = "Settings: Basic or Advanced";

for (const engine of ENGINES) {
  describe(`Basic and Advanced in ${engine}`, () => {
    for (const section of SECTIONS) {
      test(`${section}: the Basic controls are a subsequence of the Advanced ones`, async () => {
        const daemon = await startDaemon();
        const app = await openApp(engine, daemon);
        try {
          await goToSection(app.page, section);
          await setMode(app.page, "Basic");
          const basic = await interactiveElements(app.page);
          await setMode(app.page, "Advanced");
          const advanced = await interactiveElements(app.page);
          assert.ok(
            isSubsequence(basic, advanced),
            `Basic ${JSON.stringify(basic)} is not in Advanced ${JSON.stringify(advanced)}`,
          );
          assert.ok(basic.length > 0, `${section} shows some controls in Basic`);
        } finally {
          await app.close();
          await daemon.stop();
        }
      });
    }

    test("a value typed in Basic is still there after Advanced and back", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await setMode(app.page, "Basic");
        await app.page.getByLabel(TOKEN_NAME_LABEL, { exact: true }).fill(TYPED_NAME);
        await setMode(app.page, "Advanced");
        await setMode(app.page, "Basic");
        assert.equal(await app.page.getByLabel(TOKEN_NAME_LABEL, { exact: true }).inputValue(), TYPED_NAME);
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("a panel's Advanced override shows the setting keys while the global mode is Basic", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await setMode(app.page, "Basic");
        assert.equal(await app.page.locator(".rr-setting-key").count(), 0, "Basic shows no setting keys");
        await app.page.getByLabel(PANEL_MODE_LABEL).selectOption({ label: "Advanced" });
        assert.ok((await app.page.locator(".rr-setting-key").count()) > 0, "the panel's Advanced shows the keys");
        await app.page.getByLabel(PANEL_MODE_LABEL).selectOption({ label: "Follow app" });
        assert.equal(await app.page.locator(".rr-setting-key").count(), 0, "following the app is Basic again");
      } finally {
        await app.close();
        await daemon.stop();
      }
    });
  });
}

/**
 * Whether every item of `needle` appears in `haystack`, in the same order.
 * @param {string[]} needle
 * @param {string[]} haystack
 * @returns {boolean}
 */
function isSubsequence(needle, haystack) {
  let position = 0;
  for (const item of needle) {
    const found = haystack.indexOf(item, position);
    if (found === -1) {
      return false;
    }
    position = found + 1;
  }
  return true;
}
