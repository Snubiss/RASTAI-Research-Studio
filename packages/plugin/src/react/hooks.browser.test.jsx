// The state hooks in a real browser (layout frontend-and-plugins.md 2.3 and 7.5): useQuery refetches on its own
// bus events only, useCommand reports running and the last outcome, useStore re-renders on set, and useLiveBuffer
// renders at most once per animation frame however many items arrive.

import { afterEach, describe, expect, test } from "vitest";

import { createLiveBuffer, pushItem, useLiveBuffer } from "../state/live-buffer.js";
import { createStore, useStore } from "../state/store.js";
import { sampleBusEvent } from "../testing/host-contract.js";
import { createMockHost } from "../testing/mock-host.js";
import { inAct, mount } from "../testing/render.js";
import { PluginHostProvider } from "./PluginHostProvider.jsx";
import { useCommand } from "./use-command.js";
import { useQuery } from "./use-query.js";

/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("../testing/render.js").Mounted} Mounted */
/** @typedef {import("../state/live-buffer.js").LiveBuffer} LiveBuffer */
/** @typedef {import("../state/store.js").Store} Store */

/** @type {Mounted[]} */
let mounted = [];

afterEach(async () => {
  for (const page of mounted) {
    await page.unmount();
  }
  mounted = [];
});

/**
 * @param {import("react").ReactNode} element
 * @returns {Promise<Mounted>}
 */
async function show(element) {
  const page = await mount(element);
  mounted.push(page);
  return page;
}

/**
 * @param {any} result
 * @returns {ExecuteOutcome}
 */
function okOutcome(result) {
  return { ok: true, result, jobId: null, diagnostics: [], journalId: null };
}

/** Lets the mock host's answers (resolved promises) arrive and React render them. */
async function settle() {
  await inAct(() => new Promise((resolve) => setTimeout(resolve, 0)));
}

/** Waits for the next animation frames to run. */
async function nextFrames() {
  await inAct(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined)))),
  );
}

/** A mock host whose job.list answers with how many times it was called. */
function countingHost() {
  let answers = 0;
  return createMockHost("core", {
    "job.list": () => {
      answers += 1;
      return okOutcome({ jobs: [], answer: answers });
    },
  });
}

/** @param {{ label: string }} props */
function JobListAnswer({ label }) {
  const jobs = useQuery("job.list", {}, ["job.state_changed"]);
  return <output aria-label={label}>{jobs.status === "ready" ? `answer ${jobs.data.answer}` : jobs.status}</output>;
}

describe("useQuery", () => {
  test("refetches on the matching bus event only", async () => {
    const mock = countingHost();
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <JobListAnswer label="jobs" />
      </PluginHostProvider>,
    );
    await settle();
    expect(page.container.textContent).toBe("answer 1");

    await inAct(() => {
      mock.emit(sampleBusEvent("node.heartbeat", "node_01"));
      mock.emit(sampleBusEvent("job.progress", "job_01"));
    });
    await settle();
    expect(mock.calls).toHaveLength(1);
    expect(page.container.textContent).toBe("answer 1");

    await inAct(() => mock.emit(sampleBusEvent("job.state_changed", "job_01")));
    await settle();
    expect(mock.calls).toHaveLength(2);
    expect(page.container.textContent).toBe("answer 2");
  });

  test("shows loading until the first answer arrives", async () => {
    const mock = createMockHost("core", {});
    mock.host.execute = () => new Promise(() => {});
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <JobListAnswer label="jobs" />
      </PluginHostProvider>,
    );
    await settle();
    expect(page.container.textContent).toBe("loading");
  });

  test("two components with the same query share one request", async () => {
    const mock = countingHost();
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <JobListAnswer label="first" />
        <JobListAnswer label="second" />
      </PluginHostProvider>,
    );
    await settle();
    expect(page.container.textContent).toBe("answer 1answer 1");
    expect(mock.calls).toHaveLength(1);
  });

  test("stops listening once the component is gone", async () => {
    const mock = countingHost();
    const page = await mount(
      <PluginHostProvider host={mock.host}>
        <JobListAnswer label="jobs" />
      </PluginHostProvider>,
    );
    await settle();
    await page.unmount();
    mock.emit(sampleBusEvent("job.state_changed", "job_01"));
    await settle();
    expect(mock.calls).toHaveLength(1);
  });
});

function CancelButton() {
  const cancel = useCommand("job.cancel");
  return (
    <div>
      <button type="button" onClick={() => cancel.run({ job_id: "job_01" })}>
        Cancel
      </button>
      <output>{cancel.running ? "running" : cancel.outcome ? `ok ${cancel.outcome.ok}` : "idle"}</output>
    </div>
  );
}

describe("useCommand", () => {
  test("runs the command through the host and keeps the last outcome", async () => {
    /** @type {((outcome: ExecuteOutcome) => void)[]} */
    const answers = [];
    const mock = createMockHost("core", {});
    const execute = mock.host.execute;
    mock.host.execute = (commandId, args, options) => {
      execute(commandId, args, options);
      return new Promise((resolve) => answers.push(resolve));
    };
    const page = await show(
      <PluginHostProvider host={mock.host}>
        <CancelButton />
      </PluginHostProvider>,
    );
    expect(page.container.querySelector("output")?.textContent).toBe("idle");
    await inAct(() => page.container.querySelector("button")?.click());
    expect(page.container.querySelector("output")?.textContent).toBe("running");
    expect(mock.calls.map((call) => [call.commandId, call.args])).toEqual([["job.cancel", { job_id: "job_01" }]]);
    await inAct(() => answers[0](okOutcome({ job: { state: "cancelled" } })));
    expect(page.container.querySelector("output")?.textContent).toBe("ok true");
  });
});

/** @param {{ store: Store }} props */
function RouteLabel({ store }) {
  return <output>{useStore(store)}</output>;
}

describe("useStore", () => {
  test("re-renders when the store is set", async () => {
    const store = createStore("#/home");
    const page = await show(<RouteLabel store={store} />);
    expect(page.container.textContent).toBe("#/home");
    await inAct(() => store.set("#/inference"));
    expect(page.container.textContent).toBe("#/inference");
  });
});

/** @param {{ buffer: LiveBuffer, onRender: () => void }} props */
function LiveCount({ buffer, onRender }) {
  const nextSequence = useLiveBuffer(buffer);
  onRender();
  return <output>{nextSequence}</output>;
}

describe("useLiveBuffer", () => {
  test("renders at most once per frame, with everything pushed before it", async () => {
    const buffer = createLiveBuffer(64);
    let renders = 0;
    const page = await show(
      <LiveCount
        buffer={buffer}
        onRender={() => {
          renders += 1;
        }}
      />,
    );
    const rendersBefore = renders;
    for (let token = 0; token < 500; token += 1) {
      pushItem(buffer, token);
    }
    await nextFrames();
    expect(page.container.textContent).toBe("500");
    // Two frames passed, so at most two renders, never one per pushed item.
    expect(renders - rendersBefore).toBeGreaterThanOrEqual(1);
    expect(renders - rendersBefore).toBeLessThanOrEqual(2);

    const rendersAfterPushes = renders;
    await nextFrames();
    expect(renders).toBe(rendersAfterPushes);
  });
});
