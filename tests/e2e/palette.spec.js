// The command palette in a real daemon's app (plan M1-54; layout 2.5; ui-design 3.6, 6.3): it lists the daemon's commands,
// Basic mode filters them, a command runs from it, and every copy-as line equals the golden file that the CLI's tests parse.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, test } from "node:test";

import { ENGINES, goToSection, openApp, setMode, startDaemon } from "./harness.js";

const GOLDEN = JSON.parse(readFileSync(new URL("../../fixtures/command-golden/copy-as.json", import.meta.url), "utf8"));
const TOKEN_CREATE_GOLDEN = GOLDEN.examples.find((example) => example.command_id === "token.create");
const SKELETON_TITLES = ["Show status", "List jobs", "Show settings", "Stop the daemon"];
// The title the daemon gives token.create (its row in the command table).
const CREATE_TOKEN_TITLE = "Create an API token";
const TOKEN_NAME = "ci bot's token";
const TOKEN_SCOPES_TEXT = '["training", "eval"]';

for (const engine of ENGINES) {
  describe(`the palette in ${engine}`, () => {
    test("Ctrl K opens it and lists the commands the daemon registered", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await app.page.keyboard.press("Control+K");
        const titles = await paletteTitles(app.page);
        for (const title of SKELETON_TITLES) {
          assert.ok(titles.includes(title), `the palette lists "${title}": ${titles.join(", ")}`);
        }
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("Basic mode hides the advanced commands and says how many", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await setMode(app.page, "Basic");
        await app.page.keyboard.press("Control+K");
        // The mode change is a settings.set round trip; the footer's count shows the palette has the Basic list.
        await app.page.getByRole("button", { name: /more in Advanced$/ }).waitFor();
        const titles = await paletteTitles(app.page);
        assert.ok(!titles.includes("List commands"), "List commands is an advanced command");
        assert.ok(titles.includes("Show status"), "Show status is a basic command");
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("system.status runs from the palette and says it finished", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await app.page.keyboard.press("Control+K");
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("show status");
        await app.page.keyboard.press("Enter");
        await app.page.locator(".rr-toast", { hasText: "Show status finished." }).waitFor();
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("token.create's copy-as lines equal the golden file, in all four formats", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await app.page.keyboard.press("Control+K");
        const search = app.page.getByRole("combobox", { name: "Search or run a command" });
        await search.fill("create api token");
        await app.page.locator(".rr-palette__entry", { hasText: CREATE_TOKEN_TITLE }).first().click();
        // The arguments form: the name and the preset are typed or chosen, scopes is JSON text (the form sends it parsed).
        await app.page.getByLabel(/^Name/).fill(TOKEN_NAME);
        await app.page.getByLabel("Preset", { exact: true }).selectOption("mcp-default");
        await app.page.getByLabel("Scopes", { exact: true }).fill(TOKEN_SCOPES_TEXT);
        const copyAs = app.page.getByRole("group", { name: "Copy as" });
        await copyAs.getByRole("button", { name: "CLI", exact: true }).click();
        assert.equal(await copiedText(app.page), TOKEN_CREATE_GOLDEN.cli);
        await copyAs.getByRole("button", { name: "Python", exact: true }).click();
        assert.equal(await copiedText(app.page), TOKEN_CREATE_GOLDEN.python);
        await copyAs.getByRole("button", { name: "JavaScript", exact: true }).click();
        assert.equal(await copiedText(app.page), TOKEN_CREATE_GOLDEN.javascript);
        await copyAs.getByRole("button", { name: "MCP", exact: true }).click();
        assert.deepEqual(JSON.parse(await copiedText(app.page)), TOKEN_CREATE_GOLDEN.mcp);
      } finally {
        await app.close();
        await daemon.stop();
      }
    });

    test("Ctrl . copies the highlighted command's CLI line, and a command that needs input isn't run by Enter", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await goToSection(app.page, "Settings");
        await app.page.keyboard.press("Control+K");
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("show status");
        await app.page.keyboard.press("Control+.");
        await app.page.locator(".rr-palette-detail__status", { hasText: /Copied|did not work/ }).waitFor();
        assert.equal(await copiedText(app.page), "rastai system status");
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("create api token");
        await app.page.keyboard.press("Enter");
        // The form takes the focus; nothing ran, so no toast says a token was made.
        await app.page.getByLabel(/^Name/).waitFor();
        assert.equal(await app.page.locator(".rr-toast", { hasText: `${CREATE_TOKEN_TITLE} finished` }).count(), 0);
      } finally {
        await app.close();
        await daemon.stop();
      }
    });
  });
}

/**
 * The titles of the rows the palette shows now.
 * @param {import("playwright").Page} page
 * @returns {Promise<string[]>}
 */
async function paletteTitles(page) {
  await page.getByRole("listbox", { name: "Search or run a command" }).waitFor();
  return page.locator(".rr-palette__entry .rr-palette__title").allTextContents();
}

/**
 * The text of the Copy as block's code, which is what the copy writes.
 * @param {import("playwright").Page} page
 * @returns {Promise<string>}
 */
async function copiedText(page) {
  return (await page.locator(".rr-palette-detail__code code").textContent()) ?? "";
}
