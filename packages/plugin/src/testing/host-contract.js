// The shared table for the rest of the PluginHost shape (layout frontend-and-plugins.md 7.5): subscribe, mode,
// theme, display changes, navigate, setContext, contributions, showDiagnostics and the options a plugin may set.
// `execute` has its own table in @rastai/client (EXECUTOR_CONTRACT_CASES). Every PluginHost passes both: the mock
// host and the in-process host now, the iframe guest from M8. Plain data and plain functions, no test library:
// each case does one thing with a host and returns what it saw, and the test compares that with `expected`.

import { builtInTheme, LOCKED_TOKENS } from "@rastai/ui";

/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("./mock-host.js").MockHost} MockHost */
/** @typedef {import("../host-api.js").ThemeTokens} ThemeTokens */

/**
 * The plugin every case's host is built for. A host under test is a MockHost-shaped object: the host, plus the
 * levers and records of the world around it (its event hub, display, routes and the app's registries).
 */
export const HOST_CONTRACT_PLUGIN_ID = "psychology";

/**
 * @typedef {object} HostContractCase
 * @property {string} name
 * @property {(target: MockHost) => any} run   does one thing with target.host and returns what it saw
 * @property {any} expected
 */

const ANY_SUBJECT = /** @type {string[]} */ ([]);
// The CSS variable prefix of the locked tokens (@rastai/ui tokens/locked.js); a theme must never carry it.
const LOCKED_VARIABLE_PREFIX = "--rr-locked-";
const DARK_THEME = builtInTheme("dark", { density: "compact", reducedMotion: true, colorblind: false });
const SAMPLE_DIAGNOSTIC = /** @type {import("../host-api.js").Diagnostic} */ ({
  $typeName: "rastai.common.v1.Diagnostic",
  code: "RRS-J0005",
  severity: 1,
  message: "No job job_404 is known to this daemon.",
  path: "",
  fixText: "",
  fixCommandId: "",
  fixArgumentsJson: "",
});

/**
 * A bus event as the event hub delivers it, with only the fields the cases read filled.
 * @param {string} eventType
 * @param {string} subjectId
 * @returns {BusEvent}
 */
export function sampleBusEvent(eventType, subjectId) {
  return /** @type {BusEvent} */ ({
    $typeName: "rastai.events.v1.BusEvent",
    busSequence: 1n,
    eventType,
    eventClass: 0,
    subjectId,
    source: "daemon",
    emittedUnixMs: 0n,
  });
}

/**
 * Subscribes with the filter, plays the events, and returns "<type> <subject>" for each one delivered.
 * @param {MockHost} target
 * @param {string[]} eventTypes
 * @param {string[]} subjectIds
 * @param {BusEvent[]} events
 * @returns {string[]}
 */
function deliveredEvents(target, eventTypes, subjectIds, events) {
  /** @type {string[]} */
  const delivered = [];
  const unsubscribe = target.host.subscribe({ eventTypes, subjectIds }, (event) =>
    delivered.push(`${event.eventType} ${event.subjectId}`),
  );
  events.forEach((event) => target.emit(event));
  unsubscribe();
  return delivered;
}

/**
 * The locked token names a theme carries: any LOCKED_TOKENS key (light or dark) among its colors, and the locked
 * variable prefix anywhere in it. A plugin's theme must have none; the security UI's colors stay with the shell.
 * @param {ThemeTokens} theme
 * @returns {string[]}
 */
function lockedTokensIn(theme) {
  const colorNames = Object.keys(theme.colors ?? {});
  const lockedNames = [...Object.keys(LOCKED_TOKENS.light), ...Object.keys(LOCKED_TOKENS.dark)];
  const found = colorNames.filter((name) => lockedNames.includes(name));
  if (JSON.stringify(theme).includes(LOCKED_VARIABLE_PREFIX)) {
    found.push(LOCKED_VARIABLE_PREFIX);
  }
  return found;
}

/** @type {HostContractCase[]} */
export const HOST_CONTRACT_CASES = [
  {
    name: "the host carries its plugin's id",
    run: (target) => target.host.pluginId,
    expected: HOST_CONTRACT_PLUGIN_ID,
  },
  {
    name: "subscribe delivers only the event types asked for",
    run: (target) =>
      deliveredEvents(target, ["job.state_changed"], ANY_SUBJECT, [
        sampleBusEvent("job.state_changed", "job_01"),
        sampleBusEvent("node.heartbeat", "node_01"),
        sampleBusEvent("job.state_changed", "job_02"),
      ]),
    expected: ["job.state_changed job_01", "job.state_changed job_02"],
  },
  {
    name: "subscribe with subject ids delivers only those subjects",
    run: (target) =>
      deliveredEvents(
        target,
        ["job.progress", "job.state_changed"],
        ["job_02"],
        [
          sampleBusEvent("job.progress", "job_01"),
          sampleBusEvent("job.progress", "job_02"),
          sampleBusEvent("job.state_changed", "job_02"),
        ],
      ),
    expected: ["job.progress job_02", "job.state_changed job_02"],
  },
  {
    name: "nothing is delivered after unsubscribe",
    run: (target) => {
      /** @type {string[]} */
      const delivered = [];
      const unsubscribe = target.host.subscribe({ eventTypes: ["run.metrics"], subjectIds: [] }, (event) =>
        delivered.push(event.subjectId),
      );
      target.emit(sampleBusEvent("run.metrics", "run_01"));
      unsubscribe();
      target.emit(sampleBusEvent("run.metrics", "run_02"));
      return delivered;
    },
    expected: ["run_01"],
  },
  {
    name: "mode and theme report the window's display, and a change tells the listeners until they leave",
    run: (target) => {
      let changes = 0;
      const stopListening = target.host.onDisplayChange(() => {
        changes += 1;
      });
      target.setDisplay("basic", DARK_THEME);
      const seen = [target.host.mode(), target.host.theme().base, target.host.theme().spacingPx];
      stopListening();
      target.setDisplay("advanced", DARK_THEME);
      return { seen, changes };
    },
    expected: { seen: ["basic", "dark", DARK_THEME.spacingPx], changes: 1 },
  },
  {
    name: "the theme carries no locked security token",
    run: (target) => lockedTokensIn(target.host.theme()),
    expected: [],
  },
  {
    name: "navigate passes the route to the shell",
    run: (target) => {
      target.host.navigate("#/psychology/replay/conv_01");
      return target.routes;
    },
    expected: ["#/psychology/replay/conv_01"],
  },
  {
    name: "setContext passes the values to the shell",
    run: (target) => {
      target.host.setContext({ "psychology.session": "ses_01" });
      return target.contexts;
    },
    expected: [{ "psychology.session": "ses_01" }],
  },
  {
    name: "contributions lists the kind asked for",
    run: (target) => {
      target.contributions.lenses = [
        { pluginId: "psychology", id: "psychology.logit_lens", title: "Logit lens", data: {} },
      ];
      return {
        lenses: target.host.contributions("lenses").map((entry) => entry.id),
        metrics: target.host.contributions("metrics"),
      };
    },
    expected: { lenses: ["psychology.logit_lens"], metrics: [] },
  },
  {
    name: "showDiagnostics hands the diagnostics to the shell",
    run: (target) => {
      target.host.showDiagnostics([SAMPLE_DIAGNOSTIC]);
      return target.shownDiagnostics.map((diagnostics) => diagnostics.map((diagnostic) => diagnostic.code));
    },
    expected: [["RRS-J0005"]],
  },
  {
    name: "a plugin sets only the project and dry run; the acting plugin id is always its own",
    run: async (target) => {
      await target.host.execute("job.list", {}, { projectId: "prj_01", dryRun: true, actingPluginId: "core" });
      return target.calls.map((call) => call.options);
    },
    expected: [{ projectId: "prj_01", dryRun: true, actingPluginId: HOST_CONTRACT_PLUGIN_ID }],
  },
];
