// The command palette in a real daemon's app (plan M1-54; layout 2.5; ui-design 3.6, 6.3): it lists the daemon's commands,
// Basic mode filters them, a command runs from it, and every copy-as line equals the golden file that the CLI's tests parse.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, test } from "node:test";

import { ENGINES, goToSection, highlightEntry, openApp, openPalette, setMode, startDaemon } from "./harness.js";

const GOLDEN = JSON.parse(readFileSync(new URL("../../fixtures/command-golden/copy-as.json", import.meta.url), "utf8"));
const TOKEN_CREATE_GOLDEN = GOLDEN.examples.find((example) => example.command_id === "token.create");
const SKELETON_TITLES = ["Show status", "List jobs", "Show settings", "Stop the daemon"];
// The title the daemon gives token.create (its row in the command table).
const CREATE_TOKEN_TITLE = "Create an API token";
const TOKEN_NAME = "ci bot's token";
const TOKEN_SCOPES_TEXT = '["training", "eval"]';
// The status lines the palette says after a copy (frame/CommandPalette.jsx).
const COPIED_TEXT = "Copied.";
const COPIED_CLI_TEXT = "Copied the CLI line.";
const SHOW_STATUS_CLI_LINE = "rastai system status";

for (const engine of ENGINES) {
  describe(`the palette in ${engine}`, () => {
    test("Ctrl K opens it and lists the commands the daemon registered", async () => {
      const daemon = await startDaemon();
      const app = await openApp(engine, daemon);
      try {
        await openPalette(app.page);
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
        await openPalette(app.page);
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
        await openPalette(app.page);
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("show status");
        await highlightEntry(app.page, "Show status");
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
        await openPalette(app.page);
        const search = app.page.getByRole("combobox", { name: "Search or run a command" });
        await search.fill("create api token");
        await app.page.locator(".rr-palette__entry", { hasText: CREATE_TOKEN_TITLE }).first().click();
        // The arguments form: the name and the preset are typed or chosen, scopes is JSON text (the form sends it parsed).
        await app.page.getByLabel(/^Name/).fill(TOKEN_NAME);
        await app.page.getByLabel("Preset", { exact: true }).selectOption("mcp-default");
        await app.page.getByLabel("Scopes", { exact: true }).fill(TOKEN_SCOPES_TEXT);
        const copyAs = app.page.getByRole("group", { name: "Copy as" });
        await copyAs.getByRole("button", { name: "CLI", exact: true }).click();
        assert.equal(await copyAndRead(app.page, engine), TOKEN_CREATE_GOLDEN.cli);
        await copyAs.getByRole("button", { name: "Python", exact: true }).click();
        assert.equal(await copyAndRead(app.page, engine), TOKEN_CREATE_GOLDEN.python);
        await copyAs.getByRole("button", { name: "JavaScript", exact: true }).click();
        assert.equal(await copyAndRead(app.page, engine), TOKEN_CREATE_GOLDEN.javascript);
        await copyAs.getByRole("button", { name: "MCP", exact: true }).click();
        assert.deepEqual(JSON.parse(await copyAndRead(app.page, engine)), TOKEN_CREATE_GOLDEN.mcp);
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
        await openPalette(app.page);
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("show status");
        await highlightEntry(app.page, "Show status");
        await app.page.keyboard.press("Control+.");
        if (engine === "chromium") {
          await app.page.locator(".rr-palette-detail__status", { hasText: COPIED_CLI_TEXT }).waitFor();
          assert.equal(await clipboardText(app.page), SHOW_STATUS_CLI_LINE);
        } else {
          // WebKit has no clipboard permission here, so the line on the page (the CLI format's) is what is checked.
          await app.page.locator(".rr-palette-detail__status").waitFor();
          assert.equal(await codeText(app.page), SHOW_STATUS_CLI_LINE);
        }
        await app.page.getByRole("combobox", { name: "Search or run a command" }).fill("create api token");
        await highlightEntry(app.page, CREATE_TOKEN_TITLE);
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
 * The text of the Copy as block's code: the format the palette shows, which is what Copy writes.
 * @param {import("playwright").Page} page
 * @returns {Promise<string>}
 */
async function codeText(page) {
  return (await page.locator(".rr-palette-detail__code code").textContent()) ?? "";
}

/**
 * The clipboard's text, as the page reads it (Chromium has the clipboard-read permission from the harness).
 * @param {import("playwright").Page} page
 * @returns {Promise<string>}
 */
async function clipboardText(page) {
  return page.evaluate(() => navigator.clipboard.readText());
}

/**
 * Presses the palette's Copy button and returns what was copied. In Chromium that is the clipboard, and the test waits for
 * the palette to say it copied, so a failed write fails the test. WebKit has no clipboard permission here, so there the
 * text the Copy as block shows (the format's line) stands for what Copy writes.
 * @param {import("playwright").Page} page
 * @param {string} engine   "chromium" or "webkit"
 * @returns {Promise<string>}
 */
async function copyAndRead(page, engine) {
  await page.locator(".rr-palette-detail").getByRole("button", { name: "Copy", exact: true }).click();
  if (engine === "chromium") {
    await page.locator(".rr-palette-detail__status", { hasText: COPIED_TEXT }).waitFor();
    return clipboardText(page);
  }
  return codeText(page);
}
