// A section renders unchanged on the mock host (layout frontend-and-plugins.md 7.2): one section, written once
// against PluginHost and registered with registerPluginUi, is rendered on the in-process host the app uses (over a
// real DaemonClient on the client's fake daemon) and on the mock host with the same answers canned. The two pages
// must be identical, before and after a bus event and a button press.

import { connectToDaemon } from "@rastai/client";
import { createFakeDaemon } from "@rastai/client/testing/fake-daemon";
import { builtInTheme } from "@rastai/ui";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { eventMatches } from "../event-filter.js";
import { CORE_PLUGIN_ID, createInProcessHost } from "../in-process-host.js";
import { JobProgress } from "../react/JobProgress.jsx";
import { PluginHostProvider } from "../react/PluginHostProvider.jsx";
import { useCommand } from "../react/use-command.js";
import { useQuery } from "../react/use-query.js";
import { registerPluginUi, registeredPluginUi } from "../ui-registry.js";
import { sampleBusEvent } from "./host-contract.js";
import { createMockHost } from "./mock-host.js";

/** @typedef {import("./mock-host.js").MockHost} MockHost */
import { inAct, mount } from "./render.js";

/** @typedef {import("@rastai/client/testing/fake-daemon").FakeDaemon} FakeDaemon */
/** @typedef {import("../host-api.js").PluginHost} PluginHost */
/** @typedef {import("../host-api.js").EventFilter} EventFilter */
/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("./render.js").Mounted} Mounted */

// The fake daemon answers over fetch, a few promise turns per call; this many timer turns is ample even on a busy
// machine, and costs a few milliseconds.
const SETTLE_ROUNDS = 20;
// The sample job's state; job.cancel changes it, as the daemon would.
let jobState = "running";
// What both hosts answer: command id -> the command's result.
/** @type {Object<string, () => object>} */
const RESULTS = {
  "job.list": () => ({ jobs: [{ job_id: "job_01", node_id: "local", kind: "train", state: jobState }] }),
  "job.get": () => ({ job: { job_id: "job_01", node_id: "local", kind: "train", state: jobState } }),
  "job.cancel": () => {
    jobState = "cancelled";
    return { job: { job_id: "job_01", node_id: "local", kind: "train", state: jobState } };
  },
};

/** The sample section: the jobs list with a cancel button, and the first job's progress. */
function JobsSection() {
  const jobs = useQuery("job.list", {}, ["job.state_changed"]);
  const cancel = useCommand("job.cancel");
  if (jobs.status !== "ready") {
    return <p>{jobs.status}</p>;
  }
  /** @type {{ job_id: string, state: string }[]} */
  const list = jobs.data.jobs;
  return (
    <section aria-label="Jobs">
      <ul>
        {list.map((job) => (
          <li key={job.job_id}>
            {job.job_id} {job.state}
            <button type="button" onClick={() => cancel.run({ job_id: job.job_id })}>
              Cancel
            </button>
          </li>
        ))}
      </ul>
      {list.length > 0 ? <JobProgress jobId={list[0].job_id} /> : null}
    </section>
  );
}

const CORE_UI = {
  pluginId: CORE_PLUGIN_ID,
  sections: [{ id: "jobs", render: () => <JobsSection /> }],
  panels: [],
  lensViews: [],
  metricViews: [],
  settingsPages: [],
};

/** @type {FakeDaemon} */
let daemon;
/** @type {Mounted[]} */
let mounted = [];

beforeEach(() => {
  jobState = "running";
  daemon = createFakeDaemon();
  daemon.answerExecute = (request) => {
    const result = RESULTS[request.commandId];
    return result ? { ok: true, resultJson: JSON.stringify(result()) } : { ok: false };
  };
  vi.stubGlobal("fetch", daemon.fetch);
});

afterEach(async () => {
  for (const page of mounted) {
    await page.unmount();
  }
  mounted = [];
  vi.unstubAllGlobals();
});

/**
 * The in-process host as the app builds it, over a DaemonClient, plus a way to play bus events into it.
 * @returns {Promise<{ host: PluginHost, emit: (event: BusEvent) => void }>}
 */
async function appHost() {
  const client = await connectToDaemon({ baseUrl: daemon.baseUrl, credential: { kind: "cookie", sessionKey: "k" } });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  /** @type {{ filter: EventFilter, onEvent: (event: BusEvent) => void }[]} */
  let subscribers = [];
  const theme = builtInTheme("light", { density: "comfortable", reducedMotion: false, colorblind: false });
  const host = createInProcessHost(CORE_PLUGIN_ID, [], {
    executor: client,
    scopeOf: () => "",
    subscribe: (filter, onEvent) => {
      const subscriber = { filter, onEvent };
      subscribers.push(subscriber);
      return () => {
        subscribers = subscribers.filter((other) => other !== subscriber);
      };
    },
    mode: () => "advanced",
    theme: () => theme,
    onDisplayChange: () => () => {},
    navigate: () => {},
    setContext: () => {},
    contributions: () => [],
    showDiagnostics: () => {},
  });
  const emit = (/** @type {BusEvent} */ event) =>
    subscribers
      .filter((subscriber) => eventMatches(subscriber.filter, event))
      .forEach((subscriber) => subscriber.onEvent(event));
  return { host, emit };
}

/**
 * The mock host with the same answers canned.
 * @returns {MockHost}
 */
function mockHost() {
  /** @type {Object<string, () => ExecuteOutcome>} */
  const outcomes = {};
  for (const [commandId, result] of Object.entries(RESULTS)) {
    outcomes[commandId] = () => ({ ok: true, result: result(), jobId: null, diagnostics: [], journalId: null });
  }
  return createMockHost(CORE_PLUGIN_ID, outcomes);
}

/**
 * Renders the registered section on a host, as a PanelFrame does.
 * @param {PluginHost} host
 * @returns {Promise<Mounted>}
 */
async function showSection(host) {
  const section = registeredPluginUi(CORE_PLUGIN_ID)?.sections[0];
  if (!section) {
    throw new Error("the sample section isn't registered");
  }
  const page = await mount(
    <PluginHostProvider host={host}>{section.render({ path: [], query: {} })}</PluginHostProvider>,
  );
  mounted.push(page);
  return page;
}

/** Lets every answer arrive (the fake daemon's over fetch, the mock's at once) and React render them. */
async function settle() {
  for (let round = 0; round < SETTLE_ROUNDS; round += 1) {
    await inAct(() => new Promise((resolve) => setTimeout(resolve, 0)));
  }
}

/**
 * Plays the same steps on one host and returns the page after each.
 * @param {PluginHost} host
 * @param {(event: BusEvent) => void} emit
 * @returns {Promise<string[]>}
 */
async function pagesThrough(host, emit) {
  jobState = "running";
  const page = await showSection(host);
  await settle();
  const pages = [page.container.innerHTML];
  await inAct(() => page.container.querySelector("button")?.click());
  await settle();
  // The daemon announces the cancelled job's new state, as it would after job.cancel.
  await inAct(() => emit(sampleBusEvent("job.state_changed", "job_01")));
  await settle();
  pages.push(page.container.innerHTML);
  return pages;
}

describe("a section written against PluginHost", () => {
  test("renders unchanged on the mock host", async () => {
    registerPluginUi(CORE_UI);
    const app = await appHost();
    const onApp = await pagesThrough(app.host, app.emit);
    const mock = mockHost();
    const onMock = await pagesThrough(mock.host, mock.emit);

    expect(onApp[0]).toContain("job_01 running");
    expect(onApp[1]).toContain("job_01 cancelled");
    expect(onMock).toEqual(onApp);
    expect(daemon.executeRequests.map((request) => request.commandId)).toEqual(
      mock.calls.map((call) => call.commandId),
    );
  });
});
