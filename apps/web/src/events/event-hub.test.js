// The event hub (layout frontend-and-plugins.md 2.2, 8.4): one stream for the window, the union of the listeners'
// types with the durable ones, a debounced reopen with resume, and a pause while the window is hidden.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { DURABLE_EVENT_TYPES, HIDDEN_PAUSE_MS, RESUBSCRIBE_DEBOUNCE_MS } from "../constants.js";
import { createEventHub, matches } from "./event-hub.js";

/**
 * The fake opener: the openStream the hub calls, and the streams it has opened so far.
 * @typedef {object} FakeOpener
 * @property {(eventTypes: string[], afterBusSequence: bigint, handlers: any) => any} openStream
 * @property {any[]} streams   each stream opened, with what it was opened for and the handlers it was given
 */

/**
 * @returns {FakeOpener}
 */
function fakeOpener() {
  /** @type {any[]} */
  const streams = [];
  /** @param {string[]} eventTypes @param {bigint} afterBusSequence @param {any} handlers */
  const openStream = (eventTypes, afterBusSequence, handlers) => {
    const record = { eventTypes, afterBusSequence, handlers, stopped: false, last: afterBusSequence };
    streams.push(record);
    return {
      stop: () => {
        record.stopped = true;
      },
      lastBusSequence: () => record.last,
    };
  };
  return { openStream, streams };
}

/**
 * @param {string} eventType
 * @param {string} subjectId
 * @param {bigint} busSequence
 * @returns {any}
 */
function event(eventType, subjectId, busSequence) {
  return { eventType, subjectId, busSequence };
}

/** @type {FakeOpener} */
let opener;
/** @type {string[]} */
let statuses;
let resyncs = 0;

beforeEach(() => {
  vi.useFakeTimers();
  opener = fakeOpener();
  statuses = [];
  resyncs = 0;
});

afterEach(() => {
  vi.useRealTimers();
});

/** @returns {import("./event-hub.js").EventHub} */
function newHub() {
  return createEventHub({
    openStream: opener.openStream,
    onStatus: (/** @type {string} */ status) => statuses.push(status),
    onResync: () => {
      resyncs += 1;
    },
  });
}

describe("the stream's types", () => {
  test("opens with the durable types, and adds the types a listener asks for", () => {
    const hub = newHub();
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    hub.start();
    expect(opener.streams[0].eventTypes).toEqual([...new Set([...DURABLE_EVENT_TYPES, "job.progress"])].sort());
  });

  test("a listener that wants every type makes the stream ask for every type", () => {
    const hub = newHub();
    hub.subscribe({ eventTypes: [], subjectIds: [] }, () => {});
    hub.start();
    expect(opener.streams[0].eventTypes).toEqual([]);
  });

  test("a new type reopens the stream after the debounce, resuming after the last event it handed on", () => {
    const hub = newHub();
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    hub.start();
    opener.streams[0].handlers.onEvent(event("job.progress", "job_01", 41n));
    hub.subscribe({ eventTypes: ["run.metrics"], subjectIds: [] }, () => {});
    expect(opener.streams).toHaveLength(1);
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    expect(opener.streams).toHaveLength(2);
    expect(opener.streams[0].stopped).toBe(true);
    expect(opener.streams[1].eventTypes).toContain("run.metrics");
    expect(opener.streams[1].afterBusSequence).toBe(41n);
  });

  test("several changes in one burst reopen the stream once", () => {
    const hub = newHub();
    hub.start();
    hub.subscribe({ eventTypes: ["run.metrics"], subjectIds: [] }, () => {});
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    expect(opener.streams).toHaveLength(2);
  });

  test("a change that leaves the union as it was doesn't reopen the stream", () => {
    const hub = newHub();
    hub.start();
    const unsubscribe = hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    const opened = opener.streams.length;
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    unsubscribe();
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    expect(opener.streams.length).toBe(opened);
  });
});

describe("delivery", () => {
  test("each listener gets only the types it asked for", () => {
    const hub = newHub();
    /** @type {string[]} */
    const seen = [];
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, (e) => seen.push(e.subjectId));
    hub.start();
    const handlers = opener.streams[0].handlers;
    handlers.onEvent(event("job.progress", "job_01", 1n));
    handlers.onEvent(event("run.metrics", "run_01", 2n));
    handlers.onEvent(event("job.progress", "job_02", 3n));
    expect(seen).toEqual(["job_01", "job_02"]);
  });

  test("a listener with subject ids gets only those subjects", () => {
    const filter = { eventTypes: ["job.progress"], subjectIds: ["job_02"] };
    expect(matches(filter, event("job.progress", "job_01", 1n))).toBe(false);
    expect(matches(filter, event("job.progress", "job_02", 2n))).toBe(true);
    expect(matches({ eventTypes: [], subjectIds: [] }, event("anything", "x", 3n))).toBe(true);
  });

  test("nothing is delivered after the listener unsubscribes", () => {
    const hub = newHub();
    /** @type {string[]} */
    const seen = [];
    const unsubscribe = hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, (e) => seen.push(e.subjectId));
    hub.start();
    unsubscribe();
    opener.streams[0].handlers.onEvent(event("job.progress", "job_01", 1n));
    expect(seen).toEqual([]);
  });

  test("status and resync go to the owner", () => {
    newHub().start();
    opener.streams[0].handlers.onStatus("connected", null);
    opener.streams[0].handlers.onResync();
    expect(statuses).toEqual(["connected"]);
    expect(resyncs).toBe(1);
  });
});

describe("a hidden window", () => {
  test("pauses its stream after HIDDEN_PAUSE_MS, and resumes it after the last sequence it handed on", () => {
    const hub = newHub();
    hub.subscribe({ eventTypes: ["job.progress"], subjectIds: [] }, () => {});
    hub.start();
    opener.streams[0].handlers.onEvent(event("job.progress", "job_01", 7n));
    hub.setHidden(true);
    vi.advanceTimersByTime(HIDDEN_PAUSE_MS - 1);
    expect(opener.streams[0].stopped).toBe(false);
    vi.advanceTimersByTime(1);
    expect(opener.streams[0].stopped).toBe(true);
    expect(hub.isPaused()).toBe(true);
    hub.setHidden(false);
    expect(hub.isPaused()).toBe(false);
    expect(opener.streams).toHaveLength(2);
    expect(opener.streams[1].afterBusSequence).toBe(7n);
  });

  test("a window shown again before the pause keeps its stream", () => {
    const hub = newHub();
    hub.start();
    hub.setHidden(true);
    vi.advanceTimersByTime(HIDDEN_PAUSE_MS / 2);
    hub.setHidden(false);
    vi.advanceTimersByTime(HIDDEN_PAUSE_MS);
    expect(opener.streams).toHaveLength(1);
    expect(opener.streams[0].stopped).toBe(false);
  });

  test("a paused hub doesn't reopen for a new listener until it is shown", () => {
    const hub = newHub();
    hub.start();
    hub.setHidden(true);
    vi.advanceTimersByTime(HIDDEN_PAUSE_MS);
    hub.subscribe({ eventTypes: ["run.metrics"], subjectIds: [] }, () => {});
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    expect(opener.streams).toHaveLength(1);
    hub.setHidden(false);
    expect(opener.streams).toHaveLength(2);
    expect(opener.streams[1].eventTypes).toContain("run.metrics");
  });
});

describe("stop", () => {
  test("stops the stream and cancels any pending reopen", () => {
    const hub = newHub();
    hub.start();
    hub.subscribe({ eventTypes: ["run.metrics"], subjectIds: [] }, () => {});
    hub.stop();
    vi.advanceTimersByTime(RESUBSCRIBE_DEBOUNCE_MS);
    expect(opener.streams[0].stopped).toBe(true);
    expect(opener.streams).toHaveLength(1);
  });
});
