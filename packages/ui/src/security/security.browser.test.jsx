// The locked security UI (owner decision N7; layout frontend-and-plugins.md 2.10 and 4.5): each of the four
// components looks exactly the same under a built-in theme and under a hostile theme that paints every role the
// security surface's own color and blows up the type and spacing scales. The test compares two screenshots taken in
// the same run, so no stored picture can drift, and also the computed colors against LOCKED_TOKENS.

import { afterEach, beforeAll, describe, expect, test } from "vitest";
import { page } from "vitest/browser";

import { cssRgb } from "../tokens/color-math.js";
import { loadStyles } from "../testing/load-styles.js";
import { mount } from "../testing/render.js";
import { applyThemeTokens } from "../tokens/apply-theme.js";
import { COLOR_ROLES } from "../tokens/built-in-themes.js";
import { LOCKED_TOKENS } from "../tokens/locked.js";
import { builtInTheme } from "../tokens/resolve-tokens.js";
import { EgressBadge, PermissionList, SignedOutScreen, TrustPrompt } from "./index.js";

/** @typedef {import("../tokens/token-types.js").ThemeTokens} ThemeTokens */
/** @typedef {import("react").ReactElement} ReactElement */

const BASES = /** @type {const} */ (["light", "dark"]);
const OPTIONS = { density: /** @type {const} */ ("comfortable"), reducedMotion: false, colorblind: false };
const HOSTILE_FONT_SIZE_PX = 40;
const HOSTILE_SPACE_PX = 64;
const ACTIVE_EGRESS = [
  { purpose: "model download", host: "huggingface.co", subjectId: "model_01" },
  { purpose: "API chat", host: "api.example.com", subjectId: "conv_01" },
];
const PERMISSIONS = ["commands:[read, chat]", "network:[huggingface.co]", "gpu", "weights.read"];
const noop = () => {};
// The test's own frame around each component, with a fixed font, so the component's place on the page doesn't
// move with the theme's type scale. Only the component inside is compared.
const FRAME_STYLE = { display: "block", padding: "8px", fontSize: "14px", lineHeight: "20px" };

/** @type {Array<{ name: string, element: () => ReactElement, selector: string }>} */
const COMPONENTS = [
  {
    name: "EgressBadge",
    element: () => <EgressBadge activeEgress={ACTIVE_EGRESS} onOpen={noop} />,
    selector: "button",
  },
  {
    name: "PermissionList",
    element: () => <PermissionList requested={PERMISSIONS} granted={["gpu"]} />,
    selector: "ul",
  },
  {
    name: "TrustPrompt",
    element: () => <TrustPrompt pluginName="Example" permissions={PERMISSIONS} onAccept={noop} onDecline={noop} />,
    selector: "dialog",
  },
  { name: "SignedOutScreen", element: () => <SignedOutScreen />, selector: "[role=alert]" },
];

/**
 * Every role the security surface's color, and huge type and spacing: a theme trying to hide the security UI.
 * @param {"light" | "dark"} base
 * @returns {ThemeTokens}
 */
function hostileTokens(base) {
  const tokens = builtInTheme(base, OPTIONS);
  const camouflage = LOCKED_TOKENS[base]["security-surface"];
  const sizes = {
    xs: HOSTILE_FONT_SIZE_PX,
    sm: HOSTILE_FONT_SIZE_PX,
    md: HOSTILE_FONT_SIZE_PX,
    lg: HOSTILE_FONT_SIZE_PX,
    xl: HOSTILE_FONT_SIZE_PX,
  };
  return {
    ...tokens,
    colors: Object.fromEntries(COLOR_ROLES.map((role) => [role, camouflage])),
    typography: { ...tokens.typography, fontSizesPx: sizes },
    spacingPx: tokens.spacingPx.map(() => HOSTILE_SPACE_PX),
  };
}

/**
 * The page behind the component is drawn in a locked color too, so a screenshot's rounded corners don't show the
 * theme's background.
 */
function lockPageBackground() {
  document.documentElement.style.background = "var(--rr-locked-security-surface)";
}

/**
 * A screenshot of the element and the computed colors of it and everything inside it.
 * @param {Element} element
 * @returns {Promise<{ screenshot: string, colors: string[] }>}
 */
async function capture(element) {
  const screenshot = await page.screenshot({ element, save: false });
  const elements = [element, ...element.querySelectorAll("*")];
  const colors = elements.map((item) => {
    const style = getComputedStyle(item);
    return [style.color, style.backgroundColor, style.borderTopColor, style.fontSize].join(" ");
  });
  return { screenshot, colors };
}

beforeAll(async () => {
  await loadStyles();
});

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.removeAttribute("style");
});

describe("the security UI ignores the theme", () => {
  for (const base of BASES) {
    for (const component of COMPONENTS) {
      test(`${component.name} looks the same under ${base} and a hostile ${base} theme`, async () => {
        lockPageBackground();
        applyThemeTokens(document.documentElement, builtInTheme(base, OPTIONS));
        const mounted = await mount(<div style={FRAME_STYLE}>{component.element()}</div>);
        const element = document.querySelector(component.selector);
        expect(element, component.selector).not.toBeNull();
        const builtIn = await capture(/** @type {Element} */ (element));

        applyThemeTokens(document.documentElement, hostileTokens(base));
        const hostile = await capture(/** @type {Element} */ (element));

        expect(hostile.colors).toEqual(builtIn.colors);
        expect(hostile.screenshot === builtIn.screenshot, "the screenshots differ").toBe(true);
        await mounted.unmount();
      });
    }

    test(`the components draw with the ${base} locked colors`, async () => {
      applyThemeTokens(document.documentElement, hostileTokens(base));
      const mounted = await mount(<SignedOutScreen />);
      const screen = /** @type {HTMLElement} */ (document.querySelector("[role=alert]"));
      expect(getComputedStyle(screen).backgroundColor).toBe(cssRgb(LOCKED_TOKENS[base]["security-surface"]));
      expect(getComputedStyle(screen).color).toBe(cssRgb(LOCKED_TOKENS[base]["security-text"]));
      await mounted.unmount();

      const badge = await mount(<EgressBadge activeEgress={ACTIVE_EGRESS} onOpen={noop} />);
      const button = /** @type {HTMLElement} */ (badge.container.querySelector("button"));
      expect(getComputedStyle(button).backgroundColor).toBe(cssRgb(LOCKED_TOKENS[base].egress));
      expect(getComputedStyle(button).color).toBe(cssRgb(LOCKED_TOKENS[base]["egress-text"]));
      await badge.unmount();
    });
  }
});

describe("what the security components say", () => {
  test("the egress badge is off when nothing leaves the machine, and names every host when something does", async () => {
    const off = await mount(<EgressBadge activeEgress={[]} onOpen={noop} />);
    expect(off.container.childElementCount).toBe(0);
    await off.unmount();

    let opened = 0;
    const on = await mount(<EgressBadge activeEgress={ACTIVE_EGRESS} onOpen={() => (opened += 1)} />);
    const button = /** @type {HTMLButtonElement} */ (on.container.querySelector("button"));
    expect(button.textContent).toContain("huggingface.co and 1 more");
    expect(button.getAttribute("aria-label")).toBe(
      "Data is leaving this machine: huggingface.co (model download), api.example.com (API chat)",
    );
    button.click();
    expect(opened).toBe(1);
    await on.unmount();
  });

  test("the trust prompt lists the permissions, marks the sensitive ones and says Python is trusted code", async () => {
    const decisions = [];
    const mounted = await mount(
      <TrustPrompt
        pluginName="Example"
        permissions={PERMISSIONS}
        onAccept={() => decisions.push("accept")}
        onDecline={() => decisions.push("decline")}
      />,
    );
    const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
    expect(dialog.open).toBe(true);
    const text = dialog.textContent ?? "";
    expect(text).toContain("Install Example?");
    expect(text).toContain("Run commands that need these permissions: read, chat");
    expect(text).toContain("Connect to these internet hosts from its Python code: huggingface.co");
    expect(text).toContain("Use your GPU (sensitive)");
    expect(text).toContain("Read the weights of loaded models (sensitive)");
    expect(text).toContain("runs on this computer as trusted code");
    expect(document.activeElement?.textContent).toBe("Don't install");
    await mounted.unmount();
  });

  test("the signed-out screen shows RRS-U0003's catalog text and how to open Rastai again", async () => {
    const mounted = await mount(<SignedOutScreen />);
    const text = document.querySelector("[role=alert]")?.textContent ?? "";
    expect(text).toContain("You were signed out.");
    expect(text).toContain("Open Rastai again to sign in.");
    expect(text).toContain("rastai open");
    expect(text).toContain("RRS-U0003");
    await mounted.unmount();
  });
});
