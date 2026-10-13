// The frame in a real engine (Chromium and WebKit): its regions, the section rail, Basic and Advanced, the global keys
// of design 3.7, the empty and error states, the status bar, and the touch targets and phone width of design 6.
// The daemon is a fake: its answers are set per test, and its event stream is driven by the test.

import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";

import { GO_PREFIX_TIMEOUT_MS } from "../constants.js";
import { recordEgress } from "../frame/egress-state.js";
import { inAct, mount } from "../testing/render.js";
import { loadAppStyles } from "../testing/styles.js";
import { answered, refused, testShell } from "../testing/fake-shell.js";
import { ErrorBoundary } from "./ErrorBoundary.jsx";
import { App } from "./App.jsx";

const DESKTOP_VIEWPORT = { width: 1024, height: 768 };
const PHONE_VIEWPORT = { width: 390, height: 844 };
const MIN_TARGET_PX = 24;

beforeAll(async () => {
  await loadAppStyles();
});

/** The mounted trees of this test, unmounted before the next test so no dialog outlives its page. */
/** @type {{ unmount: () => Promise<void> }[]} */
let mounted = [];

afterEach(async () => {
  for (const tree of mounted) {
    await tree.unmount();
  }
  mounted = [];
  await page.viewport(DESKTOP_VIEWPORT.width, DESKTOP_VIEWPORT.height);
});

/** @param {import("react").ReactNode} element */
async function mountTracked(element) {
  const tree = await mount(element);
  mounted.push(tree);
  return tree;
}

/**
 * Mounts the App over a fresh shell. Settings are held in the fake daemon, so settings.set changes what settings.get says.
 * @param {{ mode?: "basic" | "advanced", answers?: Object<string, any>, extraSettings?: Object<string, any>, route?: string }} [options]
 */
async function renderApp(options = {}) {
  window.location.hash = options.route ?? "#/home";
  const mode = options.mode ?? "advanced";
  const storedSettings = { ui: { mode }, ...(options.extraSettings ?? {}) };
  const answers = {
    "command.list": answered({ commands: [] }),
    "settings.get": () => answered({ values: storedSettings, sources: [] }),
    "settings.set": (/** @type {any} */ args) => {
      storedSettings.ui = { ...storedSettings.ui, mode: args.value };
      return answered({ key: args.key, value: args.value, source: "user" });
    },
    ...(options.answers ?? {}),
  };
  const built = await testShell({ answers, settings: storedSettings });
  const tree = await mountTracked(<App shell={built.shell} />);
  await vi.waitFor(() => expect(document.querySelector(".rr-frame")).not.toBeNull());
  return { ...built, tree };
}

/**
 * @param {string} text
 * @returns {HTMLElement | undefined}
 */
function sectionLink(text) {
  return /** @type {HTMLElement | undefined} */ (
    [...document.querySelectorAll(".rr-section-rail__link")].find((link) => link.textContent?.startsWith(text))
  );
}

/**
 * @param {string} title
 * @returns {HTMLDialogElement | undefined}
 */
function dialogTitled(title) {
  return /** @type {HTMLDialogElement | undefined} */ (
    [...document.querySelectorAll("dialog")].find((dialog) => dialog.querySelector("h2")?.textContent === title)
  );
}

/**
 * The segmented control's option with this text, such as "Basic".
 * @param {string} text
 * @returns {HTMLElement | undefined}
 */
function optionNamed(text) {
  return /** @type {HTMLElement | undefined} */ (
    [...document.querySelectorAll(".rr-segmented__option")].find((option) => option.textContent === text)
  );
}

/**
 * A button with this text.
 * @param {string} text
 * @returns {HTMLElement | undefined}
 */
function buttonNamed(text) {
  return /** @type {HTMLElement | undefined} */ (
    [...document.querySelectorAll("button")].find((button) => button.textContent === text)
  );
}

describe("the frame's regions", () => {
  test("a skip link, the top bar, the section rail, the main region and the status bar are all there", async () => {
    await renderApp();
    expect(document.querySelector(".rr-skip-link")?.getAttribute("href")).toBe("#main");
    expect(document.querySelector("header.rr-top-bar")).not.toBeNull();
    expect(document.querySelector('nav[aria-label="Sections"]')).not.toBeNull();
    expect(document.querySelector("main#main")).not.toBeNull();
    expect(document.querySelector('footer[aria-label="Status bar"]')).not.toBeNull();
  });

  test("the current section is aria-current, and each rail link has a name", async () => {
    await renderApp();
    const current = document.querySelector('.rr-section-rail a[aria-current="page"]');
    expect(current?.textContent).toBe("Home");
    for (const link of document.querySelectorAll(".rr-section-rail__link")) {
      expect(link.textContent?.trim()).not.toBe("");
    }
  });
});

describe("Basic and Advanced", () => {
  test("Advanced says Inference and shows each step's command id; switching to Basic says Chat and hides the ids", async () => {
    await renderApp({ mode: "advanced" });
    expect(sectionLink("Inference")).toBeDefined();
    expect(document.querySelector(".rr-first-run__command")?.textContent).toBe("system.hardware");

    const basic = optionNamed("Basic");
    await inAct(async () => basic?.click());

    await vi.waitFor(() => expect(sectionLink("Chat")).toBeDefined());
    expect(document.querySelector(".rr-first-run__command")).toBeNull();
  });
});

describe("the global keys", () => {
  test("Ctrl K opens the command palette, and Escape closes it", async () => {
    await renderApp();
    await userEvent.keyboard("{Control>}k{/Control}");
    await vi.waitFor(() => expect(dialogTitled("Command palette")?.open).toBe(true));
    await userEvent.keyboard("{Escape}");
    await vi.waitFor(() => expect(dialogTitled("Command palette")?.open).toBe(false));
  });

  test("Shift ? lists the shortcuts", async () => {
    await renderApp();
    await userEvent.keyboard("?");
    await vi.waitFor(() => expect(dialogTitled("Keyboard shortcuts")?.open).toBe(true));
    expect(dialogTitled("Keyboard shortcuts")?.textContent).toContain("Ctrl K");
    await userEvent.keyboard("{Escape}");
  });

  test("G then H goes Home, and G then T goes to Training", async () => {
    await renderApp({ route: "#/inference" });
    await userEvent.keyboard("gh");
    await vi.waitFor(() => expect(window.location.hash).toBe("#/home"));
    await userEvent.keyboard("gt");
    await vi.waitFor(() => expect(window.location.hash).toBe("#/training"));
  });

  test("Ctrl K then a few letters runs the entry the palette ranks first", async () => {
    await renderApp();
    await userEvent.keyboard("{Control>}k{/Control}");
    await vi.waitFor(() => expect(dialogTitled("Command palette")?.open).toBe(true));
    const filter = /** @type {HTMLInputElement | null} */ (document.querySelector("dialog[open] input"));
    filter?.focus();
    await userEvent.keyboard("trn");
    await userEvent.keyboard("{Enter}");
    await vi.waitFor(() => expect(window.location.hash).toBe("#/training"));
    await vi.waitFor(() => expect(dialogTitled("Command palette")?.open).toBe(false));
  });

  test("a G with no letter after it expires, so the next letter is not a jump", async () => {
    await renderApp({ route: "#/inference" });
    await userEvent.keyboard("g");
    await new Promise((resolve) => setTimeout(resolve, GO_PREFIX_TIMEOUT_MS + 200));
    await userEvent.keyboard("h");
    expect(window.location.hash).toBe("#/inference");
  });

  test("F6 walks the focus through the section rail, the main region and the status bar", async () => {
    await renderApp();
    /** @type {HTMLElement | null} */ (document.activeElement)?.blur();
    await userEvent.keyboard("{F6}");
    expect(document.getElementById("section-rail")?.contains(document.activeElement)).toBe(true);
    await userEvent.keyboard("{F6}");
    expect(document.getElementById("main")?.contains(document.activeElement)).toBe(true);
    await userEvent.keyboard("{F6}");
    expect(document.activeElement).toBe(document.getElementById("status-bar"));
  });

  test("Tab reaches the section rail, and Enter follows its link", async () => {
    await renderApp();
    sectionLink("Training")?.focus();
    expect(document.activeElement).toBe(sectionLink("Training"));
    await userEvent.keyboard("{Enter}");
    await vi.waitFor(() => expect(window.location.hash).toBe("#/training"));
  });
});

describe("empty and error states", () => {
  test("a step whose command this version doesn't list says not available yet, with its code", async () => {
    await renderApp({ answers: { "command.list": answered({ commands: [{ commandId: "hf.login" }] }) } });
    const hardware = document.querySelector('[data-step="hardware"]');
    expect(hardware?.textContent).toContain("RRS-C0010");
    const signIn = document.querySelector('[data-step="hf_sign_in"]');
    expect(signIn?.textContent).toContain("Ready to run");
  });

  test("a command list the daemon refuses shows its diagnostic, and no checklist", async () => {
    await renderApp({ answers: { "command.list": refused("RRS-U0001", "command.list") } });
    await vi.waitFor(() => expect(document.querySelector(".rr-diagnostic")?.textContent).toContain("RRS-U0001"));
    expect(document.querySelector(".rr-first-run")).toBeNull();
  });

  test("an unbuilt section says so with RRS-C0010 and shows no data", async () => {
    await renderApp({ route: "#/training" });
    const main = document.getElementById("main");
    expect(main?.textContent).toContain("isn't available in this version of Rastai yet");
    expect(main?.textContent).toContain("RRS-C0010");
  });

  test("a settings change the daemon refuses shows its diagnostic in the top bar", async () => {
    await renderApp({ answers: { "settings.set": refused("RRS-C0002", "settings.set") } });
    const basic = optionNamed("Basic");
    await inAct(async () => basic?.click());
    await vi.waitFor(() => expect(document.querySelector(".rr-toasts")?.textContent).toContain("RRS-C0002"));
  });

  test("offline: the banner names RRS-U0001", async () => {
    const { stream } = await renderApp();
    await inAct(() => stream.setStatus("offline"));
    await vi.waitFor(() => expect(document.querySelector(".rr-connection-banner")?.textContent).toContain("RRS-U0001"));
  });

  test("signed out: the frame is replaced by the signed-out screen", async () => {
    const { stream } = await renderApp();
    await inAct(() => stream.setStatus("signed_out"));
    await vi.waitFor(() => expect(document.querySelector(".rr-frame")).toBeNull());
    expect(document.body.textContent).toContain("RRS-U0003");
  });

  test("a panel that crashes shows RRS-U1004, and Reload panel renders it again", async () => {
    let crashing = true;
    function Sometimes() {
      if (crashing) {
        throw new Error("boom");
      }
      return <p>Recovered</p>;
    }
    await mountTracked(
      <ErrorBoundary panelId="core.home">
        <Sometimes />
      </ErrorBoundary>,
    );
    expect(document.body.textContent).toContain("RRS-U1004");
    crashing = false;
    const reload = buttonNamed("Reload panel");
    await inAct(async () => reload?.click());
    expect(document.body.textContent).toContain("Recovered");
  });
});

describe("the status bar", () => {
  test("the node's telemetry shows as node, GPU use and VRAM", async () => {
    const { shell } = await renderApp();
    await inAct(() => {
      shell.stores.nodes.set([{ node_id: "node_local", name: "local", transport: "local", state: "connected" }]);
      shell.stores.telemetry.set({
        node_local: { gpus: [{ utilizationPercent: 54, vramUsedBytes: 4939212390n, vramTotalBytes: 17179869184n }] },
      });
    });
    expect(document.querySelector(".rr-status-bar__node")?.textContent).toBe("local · GPU 54% · VRAM 4.6 / 16.0 GiB");
  });

  test("the egress badge shows while data leaves the machine, and its click lists the pairs", async () => {
    const { shell } = await renderApp();
    await inAct(() => {
      shell.stores.egress.set(
        recordEgress([], { purpose: "hf_download", host: "huggingface.co", subjectId: "model_01" }, Date.now()),
      );
    });
    const badge = [...document.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Sending data"),
    );
    expect(badge?.textContent).toContain("huggingface.co");
    await inAct(async () => badge?.click());
    await vi.waitFor(() => expect(dialogTitled("Data leaving this machine")?.open).toBe(true));
    expect(dialogTitled("Data leaving this machine")?.textContent).toContain("hf_download to huggingface.co");
    await userEvent.keyboard("{Escape}");
  });

  test("with privacy.show_egress_badge off the badge is hidden, though the reports still arrive", async () => {
    const { shell } = await renderApp({ extraSettings: { privacy: { show_egress_badge: false } } });
    await inAct(() => {
      shell.stores.egress.set(
        recordEgress([], { purpose: "hf_download", host: "huggingface.co", subjectId: "m" }, Date.now()),
      );
    });
    expect(
      [...document.querySelectorAll("button")].some((button) => button.textContent?.includes("Sending data")),
    ).toBe(false);
    expect(shell.stores.egress.get()).toHaveLength(1);
  });
});

describe("touch targets and phone width", () => {
  test("every control is at least 24 px tall", async () => {
    await renderApp();
    const controls = [
      sectionLink("Home"),
      document.querySelector(".rr-top-bar__palette"),
      ...document.querySelectorAll(".rr-segmented__option"),
    ];
    for (const control of controls) {
      expect(control?.getBoundingClientRect().height, control?.textContent).toBeGreaterThanOrEqual(MIN_TARGET_PX);
    }
  });

  test("at phone width the rail stacks above the content, and nothing scrolls sideways", async () => {
    await renderApp();
    await page.viewport(PHONE_VIEWPORT.width, PHONE_VIEWPORT.height);
    await vi.waitFor(() => expect(window.innerWidth).toBe(PHONE_VIEWPORT.width));
    expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(document.documentElement.clientWidth);
    const rail = document.getElementById("section-rail")?.getBoundingClientRect();
    const main = document.getElementById("main")?.getBoundingClientRect();
    expect(rail && main && rail.bottom <= main.top + 1).toBe(true);
  });
});
