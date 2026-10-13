// The frame's accessibility (WCAG 2.2 AA, frontend layout 2.9): axe-core runs on the rendered frame in each view the
// shell has, in both modes, and reports no violation. A hand-written check of the regions and names sits beside it,
// because axe can't see a missing skip link or an unnamed landmark's purpose.

import axe from "axe-core";
import { beforeAll, describe, expect, test, vi } from "vitest";

import { inAct, mount } from "../testing/render.js";
import { loadAppStyles } from "../testing/styles.js";
import { answered, testShell } from "../testing/fake-shell.js";
import { App } from "./App.jsx";

const WCAG_TAGS = ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa", "best-practice"];

// axe measures targets and contrast as drawn, so the page needs its stylesheets, as main.jsx gives it.
beforeAll(async () => {
  await loadAppStyles();
});

/**
 * Mounts the App in one mode at one route, and returns the axe results for the frame.
 * @param {"basic" | "advanced"} mode
 * @param {string} route
 */
async function auditFrame(mode, route) {
  window.location.hash = route;
  const { shell } = await testShell({
    settings: { ui: { mode } },
    answers: {
      "command.list": answered({ commands: [{ commandId: "hf.login" }] }),
      "settings.get": answered({ values: { ui: { mode } }, sources: [] }),
    },
  });
  const tree = await mount(<App shell={shell} />);
  await vi.waitFor(() => expect(document.querySelector(".rr-frame")).not.toBeNull());
  await inAct(async () => {});
  const frame = /** @type {HTMLElement} */ (document.querySelector(".rr-frame"));
  const results = await axe.run(frame, { runOnly: { type: "tag", values: WCAG_TAGS } });
  await tree.unmount();
  // Each violation names the elements it found, so a failing test says where to look.
  return results.violations.map(
    (violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(" ")).join(", ")}`,
  );
}

describe("axe-core on the frame", () => {
  for (const mode of /** @type {const} */ (["basic", "advanced"])) {
    for (const route of ["#/home", "#/training"]) {
      test(`no violations in ${mode} mode at ${route}`, async () => {
        expect(await auditFrame(mode, route)).toEqual([]);
      });
    }
  }
});

describe("the frame's own landmarks", () => {
  test("the page has one main region, one banner and one navigation named Sections", async () => {
    window.location.hash = "#/home";
    const { shell } = await testShell({ settings: { ui: { mode: "advanced" } } });
    const tree = await mount(<App shell={shell} />);
    await vi.waitFor(() => expect(document.querySelector(".rr-frame")).not.toBeNull());
    expect(document.querySelectorAll("main")).toHaveLength(1);
    expect(document.querySelectorAll("header[class~='rr-top-bar']")).toHaveLength(1);
    expect(document.querySelector('nav[aria-label="Sections"]')).not.toBeNull();
    await tree.unmount();
  });
});
