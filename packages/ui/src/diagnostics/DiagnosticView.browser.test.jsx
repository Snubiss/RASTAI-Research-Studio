// DiagnosticView renders both texts from the generated catalog (layout frontend-and-plugins.md 2.4 step 6 and 2.7):
// Basic shows the catalog's basic_text and the fix button; Advanced adds the code, path, message and fix text. The
// severity color is the locked one, whatever the theme says.

import { CATALOG } from "@rastai/catalog";
import { afterEach, beforeAll, describe, expect, test } from "vitest";

import { ModeProvider } from "../mode/ModeProvider.jsx";
import { cssRgb } from "../tokens/color-math.js";
import { loadStyles } from "../testing/load-styles.js";
import { inAct, mount } from "../testing/render.js";
import { applyThemeTokens } from "../tokens/apply-theme.js";
import { COLOR_ROLES } from "../tokens/built-in-themes.js";
import { LOCKED_TOKENS } from "../tokens/locked.js";
import { builtInTheme } from "../tokens/resolve-tokens.js";
import { DiagnosticView } from "./DiagnosticView.jsx";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

// RRS-C1009 has a placeholder in its message and a fix command (the first data-policy refusal, layout 2.7).
const POLICY_CODE = "RRS-C1009";
const POLICY_ENTRY = /** @type {import("@rastai/catalog").CatalogEntry} */ (
  CATALOG.find((entry) => entry.code === POLICY_CODE)
);
const POLICY_MESSAGE = POLICY_ENTRY.message.replace("{data_policy}", "local_only");
const FIX_ARGUMENTS = '{"key":"privacy.data_policy","value":"allow_remote"}';
// rastai.common.v1.Severity
const SEVERITY_ERROR = 1;
const SEVERITY_WARNING = 2;
const OPTIONS = { density: /** @type {const} */ ("comfortable"), reducedMotion: false, colorblind: false };

/**
 * @param {Partial<Diagnostic>} fields
 * @returns {Diagnostic}
 */
function diagnostic(fields) {
  return /** @type {Diagnostic} */ ({
    $typeName: "rastai.common.v1.Diagnostic",
    code: POLICY_CODE,
    severity: SEVERITY_ERROR,
    message: POLICY_MESSAGE,
    path: "projects/demo/rastai.project.json#/privacy",
    fixText: "",
    fixCommandId: "settings.set",
    fixArgumentsJson: FIX_ARGUMENTS,
    ...fields,
  });
}

beforeAll(async () => {
  await loadStyles();
});

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.removeAttribute("style");
});

describe("DiagnosticView", () => {
  test("Basic shows the catalog's basic text and the fix button, and nothing from Advanced", async () => {
    /** @type {string[][]} */
    const fixes = [];
    const mounted = await mount(
      <ModeProvider mode="basic">
        <DiagnosticView diagnostic={diagnostic({})} onFix={(command, args) => fixes.push([command, args])} />
      </ModeProvider>,
    );
    const text = mounted.container.textContent ?? "";
    expect(text).toContain(POLICY_ENTRY.basic_text);
    expect(text).not.toContain(POLICY_CODE);
    expect(text).not.toContain(POLICY_MESSAGE);
    const button = /** @type {HTMLButtonElement} */ (mounted.container.querySelector("button"));
    expect(button.textContent).toBe(POLICY_ENTRY.fix_text);
    await inAct(() => button.click());
    expect(fixes).toEqual([["settings.set", FIX_ARGUMENTS]]);
    await mounted.unmount();
  });

  test("Advanced adds the code, title, path, the filled message and the fix text", async () => {
    const mounted = await mount(
      <ModeProvider mode="advanced">
        <DiagnosticView diagnostic={diagnostic({})} onFix={() => {}} />
      </ModeProvider>,
    );
    const text = mounted.container.textContent ?? "";
    expect(text).toContain(POLICY_ENTRY.basic_text);
    expect(text).toContain(POLICY_CODE);
    expect(text).toContain(POLICY_ENTRY.title);
    expect(text).toContain("projects/demo/rastai.project.json#/privacy");
    expect(text).toContain("The project's data policy (local_only) forbids this.");
    expect(text).toContain(POLICY_ENTRY.fix_text);
    await mounted.unmount();
  });

  test("every Basic control is still there, in the same order, in Advanced", async () => {
    const basic = await mount(
      <ModeProvider mode="basic">
        <DiagnosticView diagnostic={diagnostic({})} onFix={() => {}} />
      </ModeProvider>,
    );
    const basicButtons = [...basic.container.querySelectorAll("button")].map((button) => button.textContent);
    await basic.rerender(
      <ModeProvider mode="advanced">
        <DiagnosticView diagnostic={diagnostic({})} onFix={() => {}} />
      </ModeProvider>,
    );
    const advancedButtons = [...basic.container.querySelectorAll("button")].map((button) => button.textContent);
    expect(advancedButtons).toEqual(basicButtons);
    await basic.unmount();
  });

  test("no fix command means no fix button", async () => {
    const mounted = await mount(<DiagnosticView diagnostic={diagnostic({ fixCommandId: "" })} onFix={() => {}} />);
    expect(mounted.container.querySelector("button")).toBeNull();
    await mounted.unmount();
  });

  test("a code the catalog doesn't know shows the diagnostic's own message", async () => {
    const unknown = diagnostic({ code: "PLG-example-0001", message: "The example plugin failed.", fixCommandId: "" });
    const mounted = await mount(
      <ModeProvider mode="basic">
        <DiagnosticView diagnostic={unknown} onFix={() => {}} />
      </ModeProvider>,
    );
    expect(mounted.container.textContent).toContain("The example plugin failed.");
    await mounted.unmount();
  });

  test("the severity color is the locked one, even when the theme paints every role the same", async () => {
    for (const base of /** @type {const} */ (["light", "dark"])) {
      const tokens = builtInTheme(base, OPTIONS);
      const oneColor = Object.fromEntries(COLOR_ROLES.map((role) => [role, tokens.colors.background]));
      applyThemeTokens(document.documentElement, { ...tokens, colors: oneColor });
      const mounted = await mount(
        <>
          <DiagnosticView diagnostic={diagnostic({})} onFix={() => {}} />
          <DiagnosticView diagnostic={diagnostic({ severity: SEVERITY_WARNING })} onFix={() => {}} />
        </>,
      );
      const labels = [...mounted.container.querySelectorAll(".rr-diagnostic__severity")];
      expect(labels.map((label) => getComputedStyle(label).color)).toEqual([
        cssRgb(LOCKED_TOKENS[base]["severity-error"]),
        cssRgb(LOCKED_TOKENS[base]["severity-warning"]),
      ]);
      const boxes = [...mounted.container.querySelectorAll(".rr-diagnostic")];
      expect(getComputedStyle(boxes[0]).borderLeftColor).toBe(cssRgb(LOCKED_TOKENS[base]["severity-error"]));
      await mounted.unmount();
    }
  });
});
