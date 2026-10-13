// openEventStream against the fake daemon (layout frontend-and-plugins.md 2.2, 3.5): resume with the right
// after_bus_sequence, the backoff, RRS-C0401 and onResync, a refused session, and stop().

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { connectToDaemon } from "./connection.js";
import { RECONNECT_DELAYS_MS, STREAM_OPEN_GRACE_MS } from "./constants.js";
import { openEventStream, reconnectDelayMs } from "./event-stream.js";
import { createFakeDaemon } from "./testing/fake-daemon.js";

/** @typedef {import("./testing/fake-daemon.js").FakeDaemon} FakeDaemon */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */
/** @typedef {import("./event-stream.js").EventStreamHandlers} EventStreamHandlers */
/** @typedef {import("./event-stream.js").EventStreamStatus} EventStreamStatus */

const SUBSCRIBE_PATH = "/rastai.events.v1.EventService/Subscribe";
// Longer than the first two reconnect delays together, so a loop that wrongly kept going would have retried.
const QUIET_PERIOD_MS = 1_000;
// How long a test waits for a stream with no events to count as open: well past the grace period.
const OPEN_WAIT_MS = STREAM_OPEN_GRACE_MS * 4;

/**
 * Everything the handlers were told, in order, as short strings: "status:connected", "event:7", "resync".
 * @typedef {object} Recorder
 * @property {string[]} log
 * @property {Array<{ status: EventStreamStatus, code: string | null }>} statuses
 * @property {EventStreamHandlers} handlers
 */

/** @type {FakeDaemon} */
let daemon;

beforeEach(() => {
  daemon = createFakeDaemon();
  vi.stubGlobal("fetch", daemon.fetch);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** @returns {Promise<DaemonClient>} */
async function connect() {
  const client = await connectToDaemon({ baseUrl: daemon.baseUrl, credential: { kind: "cookie", sessionKey: "k" } });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  return client;
}

/** @returns {Recorder} */
function createRecorder() {
  /** @type {Recorder} */
  const recorder = {
    log: [],
    statuses: [],
    handlers: {
      onEvent: (event) => recorder.log.push(`event:${event.busSequence}`),
      onStatus: (status, problem) => {
        recorder.log.push(`status:${status}`);
        recorder.statuses.push({ status, code: problem === null ? null : problem.code });
      },
      onResync: () => recorder.log.push("resync"),
    },
  };
  return recorder;
}

/**
 * @param {number} sequence
 * @returns {object}   BusEvent fields
 */
function busEvent(sequence) {
  return { busSequence: BigInt(sequence), eventType: "run.metrics", subjectId: "run_01" };
}

function subscribeAttempts() {
  return daemon.requests.filter((request) => request.path === SUBSCRIBE_PATH).length;
}

/** @param {number} milliseconds */
function pause(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

describe("resume", () => {
  test("after a drop, the stream resumes after the last bus_sequence it handed on", async () => {
    const client = await connect();
    daemon.connections = [
      { events: [busEvent(5), busEvent(6)], end: "drop" },
      { events: [busEvent(7)], end: "open" },
    ];
    const recorder = createRecorder();
    const stream = openEventStream(client, ["run.metrics"], 4n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.log).toContain("event:7"));
    stream.stop();
    expect(daemon.subscribeRequests.map((request) => request.afterBusSequence)).toEqual([4n, 6n]);
    expect(daemon.subscribeRequests[1].eventTypes).toEqual(["run.metrics"]);
    expect(stream.lastBusSequence()).toBe(7n);
    expect(recorder.log).toEqual([
      "status:connecting",
      "status:connected",
      "event:5",
      "event:6",
      "status:reconnecting",
      "status:connected",
      "event:7",
    ]);
  });

  test("RRS-C0401 starts a new stream and calls onResync once it is open", async () => {
    const client = await connect();
    daemon.connections = [
      { events: [], end: "RRS-C0401" },
      { events: [busEvent(900)], end: "open" },
    ];
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 10n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.log).toContain("event:900"));
    stream.stop();
    // 0 asks the daemon for a new stream that starts with the next event (contracts 7.2).
    expect(daemon.subscribeRequests.map((request) => request.afterBusSequence)).toEqual([10n, 0n]);
    expect(recorder.statuses).toContainEqual({ status: "resyncing", code: "RRS-C0401" });
    // The daemon sent its headers before RRS-C0401, but that stream never counted as connected.
    expect(recorder.log).toEqual(["status:connecting", "status:resyncing", "resync", "status:connected", "event:900"]);
    // The refetch runs after the new stream is subscribed, so an event during it isn't missed.
    const resyncAt = recorder.log.indexOf("resync");
    expect(resyncAt).toBeGreaterThan(recorder.log.indexOf("status:resyncing"));
    expect(resyncAt).toBeLessThan(recorder.log.lastIndexOf("status:connected"));
    expect(recorder.log.filter((entry) => entry === "resync")).toHaveLength(1);
  });

  test("a drop alone never calls onResync", async () => {
    const client = await connect();
    daemon.connections = [
      { events: [busEvent(5)], end: "drop" },
      { events: [busEvent(6)], end: "open" },
    ];
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.log).toContain("event:6"));
    stream.stop();
    expect(recorder.log).not.toContain("resync");
  });
});

describe("backoff", () => {
  test("the delays follow RECONNECT_DELAYS_MS, then stay at the last one", () => {
    const delays = [1, 2, 3, 4, 5, 6, 7].map((failedAttempts) => reconnectDelayMs(failedAttempts));
    expect(delays).toEqual([250, 500, 1000, 2000, 5000, 5000, 5000]);
    expect(RECONNECT_DELAYS_MS).toEqual([250, 500, 1000, 2000, 5000]);
  });

  test("an unreachable daemon is retried on that schedule and goes offline after 30 s", async () => {
    const client = await connect();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    daemon.reachable = false;
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.advanceTimersByTimeAsync(0);
    expect(subscribeAttempts()).toBe(1);
    // 250 ms, then 500 ms more, then 1 s more: one attempt after each delay, none before.
    await vi.advanceTimersByTimeAsync(249);
    expect(subscribeAttempts()).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(subscribeAttempts()).toBe(2);
    await vi.advanceTimersByTimeAsync(500);
    expect(subscribeAttempts()).toBe(3);
    await vi.advanceTimersByTimeAsync(1000);
    expect(subscribeAttempts()).toBe(4);
    expect(recorder.statuses.at(-1)).toEqual({ status: "reconnecting", code: "RRS-U0001" });
    // Then every 2 s, then every 5 s: attempts at 3.75 s, 8.75 s, ... 28.75 s still say "reconnecting".
    await vi.advanceTimersByTimeAsync(27_000);
    expect(recorder.statuses.at(-1)).toEqual({ status: "reconnecting", code: "RRS-U0001" });
    // The attempt at 33.75 s has been failing for more than 30 s.
    await vi.advanceTimersByTimeAsync(5_000);
    expect(recorder.statuses.at(-1)).toEqual({ status: "offline", code: "RRS-U0001" });
    // Offline keeps retrying every 5 s in the background.
    const attemptsWhenOffline = subscribeAttempts();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(subscribeAttempts()).toBe(attemptsWhenOffline + 1);
    stream.stop();
  });

  test("a stream that ends right after its headers neither reports connected nor resets the backoff", async () => {
    const client = await connect();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    daemon.connections = [
      { events: [], end: "drop" },
      { events: [], end: "drop" },
      { events: [], end: "drop" },
    ];
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.advanceTimersByTimeAsync(0);
    expect(subscribeAttempts()).toBe(1);
    await vi.advanceTimersByTimeAsync(250);
    expect(subscribeAttempts()).toBe(2);
    // The second failure waits 500 ms, not 250 ms again.
    await vi.advanceTimersByTimeAsync(499);
    expect(subscribeAttempts()).toBe(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(subscribeAttempts()).toBe(3);
    expect(recorder.log).not.toContain("status:connected");
    // The fourth connection has no script, so it stays open and counts as connected after the grace period.
    await vi.advanceTimersByTimeAsync(1_000 + STREAM_OPEN_GRACE_MS);
    expect(subscribeAttempts()).toBe(4);
    expect(recorder.log.at(-1)).toBe("status:connected");
    stream.stop();
  });
});

describe("ending", () => {
  test("a session the gate refuses (401 and the Connect error as JSON) ends in signed_out after one attempt", async () => {
    const client = await connect();
    // As after a daemon restart: sessions don't survive one, so the gate refuses the stream before any envelope.
    daemon.refusal = "RRS-C1002";
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.statuses.at(-1)?.status).toBe("signed_out"));
    expect(recorder.statuses).toEqual([
      { status: "connecting", code: null },
      { status: "signed_out", code: "RRS-U0003" },
    ]);
    await pause(QUIET_PERIOD_MS);
    expect(subscribeAttempts()).toBe(1);
    stream.stop();
  });

  test("stop() closes an open stream and nothing reconnects", async () => {
    const client = await connect();
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.log).toContain("status:connected"), { timeout: OPEN_WAIT_MS });
    stream.stop();
    await pause(QUIET_PERIOD_MS);
    expect(subscribeAttempts()).toBe(1);
    expect(recorder.log).toEqual(["status:connecting", "status:connected"]);
  });

  test("stop() during the wait before a reconnect ends the loop", async () => {
    const client = await connect();
    daemon.connections = [{ events: [], end: "drop" }];
    const recorder = createRecorder();
    const stream = openEventStream(client, [], 0n, recorder.handlers);
    await vi.waitFor(() => expect(recorder.log).toContain("status:reconnecting"));
    stream.stop();
    await pause(QUIET_PERIOD_MS);
    expect(subscribeAttempts()).toBe(1);
  });
});
