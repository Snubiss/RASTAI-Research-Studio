// Accessibility of every section the window shows, in both modes and both built-in light and dark themes (layout 8.7;
// plan M1-54; spec 7): axe-core, with the WCAG 2.2 AA tags, finds no violation.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, test } from "node:test";

import { ENGINES, goToSection, openApp, setMode, startDaemon } from "./harness.js";

// axe-core is the web app's dev dependency (apps/web/package.json); its own script is the one the page loads.
const REPO_ROOT = resolve(fileURLToPath(new URL("../../", import.meta.url)));
const AXE_SCRIPT = createRequire(join(REPO_ROOT, "apps/web/package.json")).resolve("axe-core/axe.min.js");
// The page's Content Security Policy refuses a script tag (`script-src 'self'`), so the script runs through
// page.evaluate, which that policy doesn't govern; the page itself is unchanged.
const AXE_SOURCE = readFileSync(AXE_SCRIPT, "utf8");
const WCAG_TAGS = ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"];
// The rail's sections that have a page in this version (the others show the not-available state, which is checked too).
// Inference is "Chat" in Basic and "Inference" in Advanced (ui-design decision 9), so its name is a pattern.
const SECTIONS = [
  "Home",
  /Chat|Inference/,
  "Training",
  "Designer",
  "Data",
  "Agent Gym",
  "Remote",
  "Plugins",
  "Settings",
];
const COLOR_SCHEMES = ["light", "dark"];
const MODES = ["Basic", "Advanced"];

for (const engine of ENGINES) {
  describe(`accessibility in ${engine}`, () => {
    test("no WCAG 2.2 AA violation on any section, in either mode and either built-in theme", async () => {
      const daemon = await startDaemon();
      try {
        for (const colorScheme of COLOR_SCHEMES) {
          const app = await openApp(engine, daemon, { colorScheme });
          try {
            await app.page.evaluate(AXE_SOURCE);
            for (const mode of MODES) {
              await setMode(app.page, mode);
              for (const section of SECTIONS) {
                await goToSection(app.page, section);
                const violations = await axeViolations(app.page);
                assert.deepEqual(violations, [], `${section} in ${mode}, ${colorScheme}`);
              }
            }
          } finally {
            await app.close();
          }
        }
      } finally {
        await daemon.stop();
      }
    });
  });
}

/**
 * The violations axe-core finds on the page: each rule's id, its impact, and the elements it flags (their selector and the
 * start of their HTML), so a failure names the element.
 * @param {import("playwright").Page} page
 * @returns {Promise<{ id: string, impact: string | null, nodes: string[] }[]>}
 */
async function axeViolations(page) {
  return page.evaluate(async (tags) => {
    const result = await window.axe.run(document, { runOnly: { type: "tag", values: tags } });
    return result.violations.map((violation) => ({
      id: violation.id,
      impact: violation.impact,
      nodes: violation.nodes.map((node) => `${node.target.join(" ")} ${node.html.slice(0, 120)}`),
    }));
  }, WCAG_TAGS);
}
