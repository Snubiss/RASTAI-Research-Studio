// The locked security UI (owner decision N7; layout frontend-and-plugins.md 2.10 and 4.5): each of the four
// components looks exactly the same under a built-in theme and under a hostile theme that paints every role the
// security surface's own color and blows up the type and spacing scales. The test compares two screenshots taken in
// the same run, so no stored picture can drift, and also the computed colors against LOCKED_TOKENS.

import { useState } from "react";
import { afterEach, beforeAll, describe, expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";

import { cssRgb } from "../tokens/color-math.js";
import { loadStyles } from "../testing/load-styles.js";
import { mount } from "../testing/render.js";
import { applyThemeTokens } from "../tokens/apply-theme.js";
import { COLOR_ROLES } from "../tokens/built-in-themes.js";
import { LOCKED_TOKENS } from "../tokens/locked.js";
import { builtInTheme } from "../tokens/resolve-tokens.js";
import { DISABLED_BUTTON_STYLE, PRIMARY_BUTTON_STYLE } from "./locked-styles.js";
import { EgressBadge, PermissionList, SignedOutScreen, TokenConfirm, TrustPrompt } from "./index.js";

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
const TOKEN_CAN = ["Read models and runs", "Start training jobs"];
const TOKEN_CANNOT = ["Change settings", "Create other tokens"];
const TOKEN_VALUE = "rst_0123456789abcdef";
const noop = () => {};
// The test's own frame around each component. It sets no font size on purpose: the component inherits whatever
// the hostile theme's type scale puts on the page, so a security element that forgets its own fixed size changes
// the screenshot. The frame is a flex box, not a block: a block's line box takes the strut of the theme's font, which
// moved an inline badge by half a pixel under the hostile theme in WebKit. flex keeps a child's margins inside too.
const FRAME_STYLE = { display: "flex", alignItems: "flex-start", padding: "8px", lineHeight: "20px" };

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
  {
    name: "TokenConfirm",
    element: () => (
      <TokenConfirm tokenName="laptop" canDo={TOKEN_CAN} cannotDo={TOKEN_CANNOT} onApprove={noop} onCancel={noop} />
    ),
    selector: "dialog",
  },
  {
    name: "TokenConfirm after approval",
    element: () => (
      <TokenConfirm
        tokenName="laptop"
        tokenValue={TOKEN_VALUE}
        canDo={TOKEN_CAN}
        cannotDo={TOKEN_CANNOT}
        onApprove={noop}
        onCancel={noop}
      />
    ),
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
    xxl: HOSTILE_FONT_SIZE_PX,
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
  await document.fonts.ready;
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
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
  // The bundled Geist faces load before the first capture, so a late font swap can't change one side of a comparison.
  await Promise.all([document.fonts.load('16px "Geist"'), document.fonts.load('16px "Geist Mono"')]);
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

/**
 * The token prompt as the settings page drives it: approval creates the token and hands its value back once.
 * @param {object} props
 * @param {string[]} props.decisions
 */
function TokenHarness({ decisions }) {
  const [tokenValue, setTokenValue] = useState(/** @type {string | null} */ (null));
  return (
    <TokenConfirm
      tokenName="laptop"
      tokenValue={tokenValue}
      canDo={TOKEN_CAN}
      cannotDo={TOKEN_CANNOT}
      onApprove={() => {
        decisions.push("approve");
        setTokenValue(TOKEN_VALUE);
      }}
      onCancel={() => decisions.push("cancel")}
    />
  );
}

/**
 * Replaces navigator.clipboard for one test. The returned function puts the real one back.
 * @param {(text: string) => Promise<void>} writeText
 * @returns {() => void}
 */
function stubClipboard(writeText) {
  const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  return () => {
    if (original) {
      Object.defineProperty(navigator, "clipboard", original);
    } else {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  };
}

/**
 * @param {ParentNode} root
 * @param {string} text
 * @returns {HTMLButtonElement | undefined}
 */
function buttonNamed(root, text) {
  return [...root.querySelectorAll("button")].find((button) => button.textContent === text);
}

describe("the token confirmation", () => {
  test("the locked header, the title and both lists show, Cancel has the focus, and Create token is enabled at once", async () => {
    /** @type {string[]} */
    const decisions = [];
    const mounted = await mount(<TokenHarness decisions={decisions} />);
    const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
    expect(dialog.open).toBe(true);
    const text = dialog.textContent ?? "";
    expect(text).toContain("Rastai security check");
    expect(text).toContain('Create the token "laptop"?');
    expect(text).toContain("It can");
    expect(text).toContain("Read models and runs");
    expect(text).toContain("It cannot");
    expect(text).toContain("Create other tokens");
    expect(text).not.toContain(TOKEN_VALUE);
    expect(dialog.querySelector("input[type=checkbox]")).toBeNull();
    expect(document.activeElement?.textContent).toBe("Cancel");
    const glyph = dialog.querySelector("svg[aria-hidden=true]");
    expect(glyph?.parentElement?.textContent).toContain("Rastai security check");

    const create = buttonNamed(dialog, "Create token");
    expect(create?.disabled).toBe(false);
    await userEvent.click(/** @type {HTMLElement} */ (create));
    expect(decisions).toEqual(["approve"]);
    await mounted.unmount();
  });

  test("Create token is disabled once pressed, so a second click cannot approve again", async () => {
    /** @type {string[]} */
    const decisions = [];
    const mounted = await mount(
      <TokenConfirm
        tokenName="laptop"
        canDo={TOKEN_CAN}
        cannotDo={TOKEN_CANNOT}
        onApprove={() => decisions.push("approve")}
        onCancel={() => decisions.push("cancel")}
      />,
    );
    const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
    const create = /** @type {HTMLButtonElement} */ (buttonNamed(dialog, "Create token"));
    create.click();
    await expect.poll(() => create.disabled).toBe(true);
    expect(getComputedStyle(create).borderTopStyle).toBe("dashed");
    // A disabled button fires no click, so the plain click() is the second click a double click would send.
    create.click();
    expect(decisions).toEqual(["approve"]);
    await mounted.unmount();
  });

  test("Copy puts the token on the clipboard and says so, and Done closes the prompt after approval", async () => {
    /** @type {string[]} */
    const written = [];
    const restore = stubClipboard(async (text) => {
      written.push(text);
    });
    try {
      /** @type {string[]} */
      const decisions = [];
      const mounted = await mount(<TokenHarness decisions={decisions} />);
      const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
      await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Create token")));
      await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Copy")));
      expect(written).toEqual([TOKEN_VALUE]);
      await expect.poll(() => dialog.querySelector("[role=status]")?.textContent).toBe("Copied.");

      await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Done")));
      expect(decisions).toEqual(["approve", "cancel"]);
      await mounted.unmount();
    } finally {
      restore();
    }
  });

  test("when the clipboard refuses, the prompt says to copy the token by hand", async () => {
    const restore = stubClipboard(async () => {
      throw new Error("clipboard blocked");
    });
    try {
      const mounted = await mount(<TokenHarness decisions={[]} />);
      const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
      await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Create token")));
      await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Copy")));
      await expect
        .poll(() => dialog.querySelector("[role=status]")?.textContent)
        .toBe("Copy did not work. Select the token and copy it by hand.");
      await mounted.unmount();
    } finally {
      restore();
    }
  });

  test("after approval the show-once box shows the new token, and Copy takes the focus", async () => {
    /** @type {string[]} */
    const decisions = [];
    const mounted = await mount(<TokenHarness decisions={decisions} />);
    const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog"));
    await userEvent.click(/** @type {HTMLElement} */ (buttonNamed(dialog, "Create token")));

    expect(dialog.querySelector("code")?.textContent).toBe(TOKEN_VALUE);
    expect(dialog.textContent).toContain("shown once");
    expect(buttonNamed(dialog, "Create token")).toBeUndefined();
    const copy = buttonNamed(dialog, "Copy");
    expect(copy).toBeDefined();
    expect(document.activeElement).toBe(copy);
    await mounted.unmount();
  });

  test("Escape declines, and the focus ring is drawn by the component", async () => {
    /** @type {string[]} */
    const decisions = [];
    const mounted = await mount(
      <TokenConfirm
        tokenName="laptop"
        canDo={TOKEN_CAN}
        cannotDo={TOKEN_CANNOT}
        onApprove={() => decisions.push("approve")}
        onCancel={() => decisions.push("cancel")}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(decisions).toEqual(["cancel"]);
    const dialogStyle = document.querySelector("dialog style");
    expect(dialogStyle?.textContent).toContain(".rr-locked:focus-visible");
    expect(dialogStyle?.textContent).toContain("var(--rr-locked-security-text");
    await mounted.unmount();
  });
});

describe("the locked buttons", () => {
  test("a disabled button is an outlined dashed button, not a copy of the filled enabled one", async () => {
    applyThemeTokens(document.documentElement, builtInTheme("light", OPTIONS));
    const mounted = await mount(
      <div>
        <button type="button" className="rr-locked" style={PRIMARY_BUTTON_STYLE}>
          Approve
        </button>
        <button type="button" className="rr-locked" style={DISABLED_BUTTON_STYLE} disabled>
          Approve
        </button>
      </div>,
    );
    const [enabled, disabled] = /** @type {HTMLElement[]} */ ([...mounted.container.querySelectorAll("button")]);
    const enabledStyle = getComputedStyle(enabled);
    const disabledStyle = getComputedStyle(disabled);
    expect(disabledStyle.backgroundColor).not.toBe(enabledStyle.backgroundColor);
    expect(disabledStyle.backgroundColor).toBe(cssRgb(LOCKED_TOKENS.light["security-surface"]));
    expect(disabledStyle.color).toBe(cssRgb(LOCKED_TOKENS.light["security-text"]));
    expect(disabledStyle.borderTopStyle).toBe("dashed");
    expect(enabledStyle.borderTopStyle).toBe("solid");
    await mounted.unmount();
  });
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
    expect(text).toContain("Rastai security check");
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

describe("SignedOutScreen's code", () => {
  test("a rejected sign-in link shows RRS-U0002's text and code; without a code it is RRS-U0003", async () => {
    const rejected = await mount(<SignedOutScreen code="RRS-U0002" />);
    expect(rejected.container.textContent).toContain("This sign-in link no longer works.");
    expect(rejected.container.textContent).toContain("RRS-U0002");
    await rejected.unmount();

    const plain = await mount(<SignedOutScreen />);
    expect(plain.container.textContent).toContain("RRS-U0003");
    await plain.unmount();
  });
});
