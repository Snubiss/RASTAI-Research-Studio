// The web app's end-to-end harness (layout frontend-and-plugins.md 8.7; plan M1-54): one `rastai serve --headless` in a
// temporary RASTAI_HOME, a sign-in link from `rastai open --print-url`, and Playwright in Chromium and WebKit opening the
// built app the daemon serves. The daemon is the real one; nothing in the page is faked. Every process the test started
// is stopped when the test ends, however it ends.

import { execFile, spawn } from "node:child_process";
import { mkdtempSync, openSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import { chromium, webkit } from "playwright";

const execFileAsync = promisify(execFile);

// This file sits in tests/e2e/, two folders below the repository root.
const REPO_ROOT = resolve(fileURLToPath(new URL("../../", import.meta.url)));
// `just e2e-web` builds the binary first. CARGO_TARGET_DIR is cargo's own variable for a target folder elsewhere.
const TARGET_FOLDER = process.env.CARGO_TARGET_DIR ?? join(REPO_ROOT, "target");
const RASTAI_BINARY = resolve(TARGET_FOLDER, "debug", "rastai");
// A short folder name keeps the daemon's socket paths within the 107 bytes a Unix socket address holds.
const HOME_PREFIX = "rw-";
const HOME_VARIABLE = "RASTAI_HOME";
const PORT_VARIABLE = "RASTAI_DAEMON__PORT";
const ANY_FREE_PORT = "0";
const PORT_FILE = "runtime/daemon.port";
const SERVE_LOG_FILE = "serve.log";
const HEALTH_PATH = "/api/health";
const START_LIMIT_MS = 120_000;
const POLL_MS = 200;
const SHUTDOWN_LIMIT_MS = 30_000;
const CLI_LIMIT_MS = 60_000;
const VIEWPORT = { width: 1024, height: 768 };
const LOGIN_CODE_PREFIX = "#login=";
// How many times Ctrl K is pressed before the palette is taken to be stuck. The window's key listener is attached after its
// first render, so under load a key pressed as soon as the frame shows can come too early; a press that finds the palette
// already open does nothing but clear its query.
const PALETTE_OPEN_ATTEMPTS = 5;
const PALETTE_OPEN_WAIT_MS = 2000;

/** The engines every spec runs in (ui-design, plan M1-54: Chromium and WebKit). */
export const ENGINES = /** @type {const} */ (["chromium", "webkit"]);
const LAUNCHERS = { chromium, webkit };

/**
 * @typedef {object} RunningDaemon
 * @property {string} baseUrl       http://127.0.0.1:<port>
 * @property {() => Promise<string>} signInUrl   a fresh sign-in link; each code works once, for a minute
 * @property {(args: string[]) => Promise<string>} cli   runs `rastai <args> --no-start` against this daemon; the stdout
 * @property {() => Promise<void>} stop
 */

/**
 * Starts `rastai serve --headless` in a new temporary home and waits until its health endpoint answers.
 * @returns {Promise<RunningDaemon>}
 */
export async function startDaemon() {
  const home = mkdtempSync(join(tmpdir(), HOME_PREFIX));
  const environment = surfaceEnvironment(home);
  const log = openSync(join(home, SERVE_LOG_FILE), "w");
  const child = spawn(RASTAI_BINARY, ["serve", "--headless"], {
    cwd: home,
    env: environment,
    stdio: ["ignore", log, log],
    detached: true,
  });
  const port = await waitForPort(home, child);
  const baseUrl = `http://127.0.0.1:${port}`;
  return {
    baseUrl,
    signInUrl: () => freshSignInUrl(environment),
    cli: (args) => runCli(environment, args),
    stop: () => stopDaemon(home, child, environment),
  };
}

/**
 * Opens the app in a new page, signed in, once the frame has drawn.
 * @param {(typeof ENGINES)[number]} engine
 * @param {RunningDaemon} daemon
 * @param {{ colorScheme?: "light" | "dark", viewport?: { width: number, height: number }, reducedMotion?: "reduce" | "no-preference" }} [options]
 *   reducedMotion sets the OS preference the page reads; "reduce" makes every transition duration 0 (base.css).
 * @returns {Promise<{ page: import("playwright").Page, close: () => Promise<void> }>}
 */
export async function openApp(engine, daemon, options = {}) {
  const browser = await LAUNCHERS[engine].launch();
  const context = await browser.newContext({
    colorScheme: options.colorScheme ?? "light",
    viewport: options.viewport ?? VIEWPORT,
    reducedMotion: options.reducedMotion ?? "no-preference",
  });
  if (engine === "chromium") {
    await grantClipboard(context, daemon.baseUrl);
  }
  const page = await context.newPage();
  await page.goto(await daemon.signInUrl());
  await page.locator(".rr-frame").waitFor({ timeout: START_LIMIT_MS });
  return {
    page,
    close: () => browser.close(),
  };
}

/**
 * The environment every `rastai` the harness runs gets: its own home, any free port, and no stray token.
 * @param {string} home
 * @returns {NodeJS.ProcessEnv}
 */
function surfaceEnvironment(home) {
  const environment = { ...process.env, [HOME_VARIABLE]: home, [PORT_VARIABLE]: ANY_FREE_PORT };
  delete environment.RASTAI_TOKEN;
  delete environment.RASTAI_URL;
  return environment;
}

/**
 * The port the daemon wrote once it is healthy.
 * @param {string} home
 * @param {import("node:child_process").ChildProcess} child
 * @returns {Promise<number>}
 */
async function waitForPort(home, child) {
  const deadline = Date.now() + START_LIMIT_MS;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`rastai serve exited with ${child.exitCode}:\n${serveLog(home)}`);
    }
    const port = readPort(home);
    if (port !== 0 && (await healthAnswers(port))) {
      return port;
    }
    await sleep(POLL_MS);
  }
  throw new Error(`rastai serve wasn't healthy within ${START_LIMIT_MS} ms:\n${serveLog(home)}`);
}

/**
 * @param {string} home
 * @returns {number} the port, or 0 before the daemon wrote one
 */
function readPort(home) {
  try {
    const text = readFileSync(join(home, PORT_FILE), "utf8").trim();
    return /^\d+$/.test(text) ? Number(text) : 0;
  } catch {
    return 0;
  }
}

/**
 * @param {number} port
 * @returns {Promise<boolean>}
 */
async function healthAnswers(port) {
  try {
    const response = await fetch(`http://127.0.0.1:${port}${HEALTH_PATH}`);
    return response.ok;
  } catch {
    return false;
  }
}

/**
 * A sign-in link from `rastai open --print-url`: the daemon is already running, so nothing else starts.
 * @param {NodeJS.ProcessEnv} environment
 * @returns {Promise<string>}
 */
async function freshSignInUrl(environment) {
  const { stdout } = await execFileAsync(RASTAI_BINARY, ["open", "--print-url", "--no-start"], {
    env: environment,
    timeout: CLI_LIMIT_MS,
  });
  const launch = JSON.parse(stdout.trim().split("\n").at(-1) ?? "{}");
  if (typeof launch.url !== "string" || !launch.url.includes(LOGIN_CODE_PREFIX)) {
    throw new Error(`rastai open --print-url gave no sign-in link: ${stdout}`);
  }
  return launch.url;
}

/**
 * Runs the CLI against the running daemon. The daemon is already up, so the CLI does not start one.
 * @param {NodeJS.ProcessEnv} environment
 * @param {string[]} args
 * @returns {Promise<string>} the command's stdout
 */
async function runCli(environment, args) {
  const { stdout } = await execFileAsync(RASTAI_BINARY, [...args, "--no-start"], {
    env: environment,
    timeout: CLI_LIMIT_MS,
  });
  return stdout;
}

/**
 * The JSON objects a `--json` command printed, one per line (the last line is the command's exit record).
 * @param {string} stdout
 * @returns {any[]}
 */
export function jsonLines(stdout) {
  return stdout
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line));
}

/**
 * Lets the page read and write the clipboard. Chromium grants it by name; WebKit has no such permission, so its tests
 * check the text on the page instead.
 * @param {import("playwright").BrowserContext} context
 * @param {string} origin
 */
async function grantClipboard(context, origin) {
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin });
}

/**
 * Asks the daemon to stop, then kills what is left of its process group and removes its home.
 * @param {string} home
 * @param {import("node:child_process").ChildProcess} child
 * @param {NodeJS.ProcessEnv} environment
 */
async function stopDaemon(home, child, environment) {
  try {
    await execFileAsync(RASTAI_BINARY, ["system", "shutdown", "--no-start", "--json"], {
      env: environment,
      timeout: CLI_LIMIT_MS,
    });
  } catch {
    // The daemon may already be gone; the group kill below is the fallback.
  }
  const deadline = Date.now() + SHUTDOWN_LIMIT_MS;
  while (child.exitCode === null && Date.now() < deadline) {
    await sleep(POLL_MS);
  }
  if (child.exitCode === null && child.pid !== undefined) {
    process.kill(-child.pid, "SIGKILL");
  }
  rmSync(home, { recursive: true, force: true });
}

/**
 * @param {string} home
 * @returns {string}
 */
function serveLog(home) {
  try {
    return readFileSync(join(home, SERVE_LOG_FILE), "utf8").slice(-4000);
  } catch {
    return "(no serve.log)";
  }
}

/** @param {number} milliseconds */
function sleep(milliseconds) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds));
}

/**
 * Moves to a section from the rail (design 3.1), and waits until the rail shows it as the current section. The rail's links
 * say the section's name, Basic or Advanced.
 * @param {import("playwright").Page} page
 * @param {string | RegExp} name   the link's text, such as "Settings", or a pattern
 */
export async function goToSection(page, name) {
  await page.locator(".rr-section-rail__link", { hasText: name }).first().click();
  // The route changes asynchronously: the page is the new section's once its rail link is the current one.
  await page.locator(".rr-section-rail__link--current", { hasText: name }).waitFor();
  await page.locator("main#main").waitFor();
}

/**
 * Switches the global Basic or Advanced mode with the top bar's switch (design 3.2), and waits until it shows the mode.
 * @param {import("playwright").Page} page
 * @param {"Basic" | "Advanced"} label
 */
export async function setMode(page, label) {
  await page
    .getByRole("group", { name: "Mode", exact: true })
    .getByRole("button", { name: label, exact: true })
    .click();
  // The change is a settings.set round trip; the switch shows the new mode only once it has landed.
  await page.waitForFunction(
    (text) =>
      [...document.querySelectorAll(".rr-top-bar .rr-segmented__option")].some(
        (button) => button.textContent === text && button.getAttribute("aria-pressed") === "true",
      ),
    label,
  );
}

/**
 * The interactive elements of the page's main region, in order, as "role: accessible name". The Basic/Advanced rule
 * compares two of these lists (layout 2.4): what Basic shows must appear, in the same order, in what Advanced shows.
 * The role is the ARIA role a control has (its role attribute, else the role its tag and type give it), and the name
 * is what a screen reader would say: aria-label, then aria-labelledby, then the label element, then the text.
 * @param {import("playwright").Page} page
 * @returns {Promise<string[]>}
 */
export async function interactiveElements(page) {
  return page.locator("main#main").evaluate((main) => {
    const INPUT_ROLES = {
      checkbox: "checkbox",
      radio: "radio",
      range: "slider",
      button: "button",
      submit: "button",
      reset: "button",
      image: "button",
    };
    /** @param {Element} element */
    const roleOf = (element) => {
      const explicit = element.getAttribute("role");
      if (explicit) {
        return explicit;
      }
      if (element.tagName === "A") {
        return "link";
      }
      if (element.tagName === "BUTTON") {
        return "button";
      }
      if (element.tagName === "SELECT") {
        return "combobox";
      }
      if (element.tagName === "TEXTAREA") {
        return "textbox";
      }
      const type = (element.getAttribute("type") ?? "text").toLowerCase();
      return INPUT_ROLES[type] ?? "textbox";
    };
    /** @param {Element} element */
    const nameOf = (element) => {
      const ariaLabel = (element.getAttribute("aria-label") ?? "").trim();
      if (ariaLabel !== "") {
        return ariaLabel;
      }
      const labelledBy = (element.getAttribute("aria-labelledby") ?? "").trim();
      if (labelledBy !== "") {
        return labelledBy
          .split(/\s+/)
          .map((id) => document.getElementById(id)?.textContent?.trim() ?? "")
          .join(" ")
          .trim();
      }
      const labels = [...(element.labels ?? [])]
        .map((label) => label.textContent?.trim() ?? "")
        .join(" ")
        .trim();
      if (labels !== "") {
        return labels;
      }
      return (element.textContent ?? "").trim() || element.getAttribute("title") || "";
    };
    const elements = [...main.querySelectorAll("a[href], button, input, select, textarea")];
    return elements
      .filter((element) => element.getClientRects().length > 0)
      .map((element) => `${roleOf(element)}: ${nameOf(element)}`);
  });
}

/**
 * Opens the command palette with Ctrl K, and waits until its search box shows.
 * @param {import("playwright").Page} page
 */
export async function openPalette(page) {
  for (let attempt = 0; attempt < PALETTE_OPEN_ATTEMPTS; attempt += 1) {
    await page.keyboard.press("Control+K");
    try {
      await page.getByRole("combobox", { name: "Search or run a command" }).waitFor({ timeout: PALETTE_OPEN_WAIT_MS });
      return;
    } catch {
      // Not open yet: press again.
    }
  }
  throw new Error(`the command palette did not open after ${PALETTE_OPEN_ATTEMPTS} presses of Ctrl K`);
}

/**
 * Waits until the palette's highlighted row is the entry with this text, so an Enter that follows runs that entry.
 * @param {import("playwright").Page} page
 * @param {string} title
 */
export async function highlightEntry(page, title) {
  await page.locator(".rr-palette__entry--active", { hasText: title }).waitFor();
}
