// The frame in a real engine (Chromium and WebKit): its regions, the section rail, Basic and Advanced, the global keys
// of design 3.7, the empty and error states, the status bar, and the touch targets and phone width of design 6.
// The daemon is a fake: its answers are set per test, and its event stream is driven by the test.

import { JobState } from "@rastai/proto";
import { afterEach, beforeAll, describe, expect, test, vi } from "vitest";
import { page, userEvent } from "vitest/browser";

import { GO_PREFIX_TIMEOUT_MS } from "../constants.js";
import { recordEgress } from "../frame/egress-state.js";
import { inAct, mount } from "../testing/render.js";
import { loadAppStyles } from "../testing/styles.js";
import { answered, busEvent, refused, testShell } from "../testing/fake-shell.js";
import { ErrorBoundary } from "./ErrorBoundary.jsx";
import { App } from "./App.jsx";

const DESKTOP_VIEWPORT = { width: 1024, height: 768 };
const PHONE_VIEWPORT = { width: 390, height: 844 };
const MIN_TARGET_PX = 24;
// Tab from the top of the page reaches the rail well inside this many presses (skip link, top bar, then the rail).
const MAX_TAB_PRESSES = 30;

beforeAll(async () => {
  await loadAppStyles();
});

/**
 * The fake daemon's running train jobs, one per count, each with its run.
 * @param {number} count
 */
function runningTrainJobs(count) {
  return Array.from({ length: count }, (_, index) => ({
    job_id: `job_0${index + 1}`,
    node_id: "node_local",
    kind: "train",
    run_id: `run_0${index + 1}`,
    state: "running",
  }));
}

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
    const training = sectionLink("Training");
    // The rail is reached by Tab from the top of the page, so the test presses Tab, as a keyboard user does.
    for (let presses = 0; presses < MAX_TAB_PRESSES && document.activeElement !== training; presses++) {
      await userEvent.keyboard("{Tab}");
    }
    expect(document.activeElement).toBe(training);
    await userEvent.keyboard("{Enter}");
    await vi.waitFor(() => expect(window.location.hash).toBe("#/training"));
  });
});

describe("empty and error states", () => {
  test("a step whose command this version doesn't list says not available yet, with its code", async () => {
    await renderApp({ answers: { "command.list": answered({ commands: [{ command_id: "hf.login" }] }) } });
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

describe("the status bar from the daemon", () => {
  test("at boot the node and the running jobs show, with no event", async () => {
    await renderApp({
      answers: {
        "node.list": answered({ nodes: [{ node_id: "node_local", name: "local", transport: "local" }] }),
        "job.list": answered({
          jobs: [{ job_id: "job_01", node_id: "node_local", kind: "train", run_id: "run_01", state: "running" }],
        }),
      },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__node")?.textContent).toContain("local"));
    expect(document.querySelector(".rr-status-bar__jobs")?.textContent).toBe("1 running");
    expect(document.querySelector(".rr-status-bar__job")?.textContent).toContain("train · run_01");
  });

  test("a node.heartbeat event is kept and shown as the node's GPU and VRAM", async () => {
    const { stream } = await renderApp({
      answers: { "node.list": answered({ nodes: [{ node_id: "node_local", name: "local" }] }) },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__node")?.textContent).toContain("local"));
    const sample = { gpus: [{ utilizationPercent: 54, vramUsedBytes: 4939212390n, vramTotalBytes: 17179869184n }] };
    await inAct(() =>
      stream.emit(busEvent("node.heartbeat", "node_local", { payload: { case: "nodeHeartbeat", value: sample } })),
    );
    expect(document.querySelector(".rr-status-bar__node")?.textContent).toBe("local · GPU 54% · VRAM 4.6 / 16.0 GiB");
  });

  test("job.progress draws a progress bar for each running job, named by its kind", async () => {
    const { stream } = await renderApp({
      answers: {
        "job.list": answered({
          jobs: [{ job_id: "job_01", node_id: "node_local", kind: "train", run_id: "", state: "running" }],
        }),
      },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    const progress = { payload: { case: "jobProgress", value: { fraction: 0.25, message: "tokenizing" } } };
    await inAct(() => stream.emit(busEvent("job.progress", "job_01", progress)));
    const bar = /** @type {HTMLProgressElement | null} */ (
      document.querySelector('progress[aria-label="train progress"]')
    );
    expect(bar?.value).toBe(0.25);
  });
});

describe("the running jobs on the status bar", () => {
  test("a progress event is keyed by the job in its source, so a run-id subject still reaches its job's bar", async () => {
    const { stream } = await renderApp({
      answers: { "job.list": answered({ jobs: runningTrainJobs(1) }) },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    const progress = { payload: { case: "jobProgress", value: { fraction: 0.5, message: "" } } };
    await inAct(() => stream.emit(busEvent("job.progress", "run_01", progress, "job:job_01")));
    const bar = /** @type {HTMLProgressElement | null} */ (
      document.querySelector('progress[aria-label="train · run_01 progress"]')
    );
    expect(bar?.value).toBe(0.5);
  });

  test("a resync drops the progress of a job the refreshed list no longer holds", async () => {
    // The daemon may have lost the final job.state_changed while the stream was down, so the refreshed list is the
    // truth: a job it no longer lists keeps no progress bar.
    let runningJobs = [{ job_id: "job_01", node_id: "node_local", kind: "train", run_id: "", state: "running" }];
    const { stream } = await renderApp({
      answers: { "job.list": () => answered({ jobs: runningJobs }) },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    const progress = { payload: { case: "jobProgress", value: { fraction: 0.25, message: "" } } };
    await inAct(() => stream.emit(busEvent("job.progress", "job_01", progress)));
    expect(document.querySelector('progress[aria-label="train progress"]')).not.toBeNull();

    runningJobs = [];
    await inAct(() => stream.opened.at(-1)?.handlers.onResync());
    await vi.waitFor(() => expect(document.querySelector('progress[aria-label="train progress"]')).toBeNull());
    expect(document.querySelector(".rr-status-bar__jobs")?.textContent).toBe("No jobs running");
  });

  test("the bar lists three running jobs and counts the rest", async () => {
    await renderApp({ answers: { "job.list": answered({ jobs: runningTrainJobs(5) }) } });
    await vi.waitFor(() => expect(document.querySelectorAll(".rr-status-bar__job")).toHaveLength(3));
    expect(document.querySelector(".rr-status-bar__jobs")?.textContent).toBe("5 running");
    expect(document.querySelector(".rr-status-bar__more")?.textContent).toBe("2 more running");
  });

  test("with three or fewer running jobs nothing is counted as more", async () => {
    await renderApp({ answers: { "job.list": answered({ jobs: runningTrainJobs(2) }) } });
    await vi.waitFor(() => expect(document.querySelectorAll(".rr-status-bar__job")).toHaveLength(2));
    expect(document.querySelector(".rr-status-bar__more")).toBeNull();
  });

  test("clicking a running job opens its run's page in its section", async () => {
    await renderApp({ answers: { "job.list": answered({ jobs: runningTrainJobs(1) }) } });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    await inAct(async () => /** @type {HTMLButtonElement} */ (document.querySelector(".rr-status-bar__job")).click());
    expect(window.location.hash).toBe("#/training/run/run_01");
  });

  test("a running job with no run opens its section's page", async () => {
    const job = { job_id: "job_09", node_id: "node_local", kind: "train", run_id: "", state: "running" };
    await renderApp({ answers: { "job.list": answered({ jobs: [job] }) } });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    await inAct(async () => /** @type {HTMLButtonElement} */ (document.querySelector(".rr-status-bar__job")).click());
    expect(window.location.hash).toBe("#/training");
  });

  test("a job that ends loses its progress bar", async () => {
    const { stream } = await renderApp({ answers: { "job.list": answered({ jobs: runningTrainJobs(1) }) } });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    const progress = { payload: { case: "jobProgress", value: { fraction: 0.5, message: "" } } };
    await inAct(() => stream.emit(busEvent("job.progress", "run_01", progress, "job:job_01")));
    expect(document.querySelector("progress")).not.toBeNull();
    const ended = {
      payload: {
        case: "jobStateChanged",
        value: { jobId: "job_01", fromState: JobState.RUNNING, toState: JobState.SUCCEEDED },
      },
    };
    await inAct(() => stream.emit(busEvent("job.state_changed", "job_01", ended, "job:job_01")));
    expect(document.querySelector("progress")).toBeNull();
  });
});

describe("the panel menu", () => {
  test("it offers only what does something in this version: Reset layout", async () => {
    await renderApp({ route: "#/home" });
    const trigger = /** @type {HTMLButtonElement | null} */ (document.querySelector(".rr-panel-chrome__menu button"));
    await inAct(async () => trigger?.click());
    const menu = document.querySelector(".rr-menu")?.textContent ?? "";
    expect(menu).toContain("Reset layout");
    expect(menu).not.toContain("Move to");
    expect(menu).not.toContain("New tab");
  });
});

describe("touch targets and phone width", () => {
  // One running job puts a status bar job button on the page, so its height is checked with the rest.
  test.each(["comfortable", "compact"])("every control is at least 24 px tall in %s density", async (density) => {
    await renderApp({
      extraSettings: { ui: { mode: "advanced", density } },
      answers: { "job.list": answered({ jobs: runningTrainJobs(1) }) },
    });
    await vi.waitFor(() => expect(document.querySelector(".rr-status-bar__job")).not.toBeNull());
    const controls = [
      sectionLink("Home"),
      document.querySelector(".rr-top-bar__palette"),
      document.querySelector(".rr-status-bar__job"),
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
