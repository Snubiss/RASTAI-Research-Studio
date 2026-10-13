// The query cache behind useQuery (layout frontend-and-plugins.md 2.3 and 7.5): a query is fetched again on its
// own events only, components share an entry, the last watcher drops it, and an older answer never wins. The hook
// itself is checked in a real browser in react/hooks.browser.test.jsx.

import { describe, expect, test } from "vitest";

import { sampleBusEvent } from "../testing/host-contract.js";
import { createMockHost } from "../testing/mock-host.js";
import { canonicalJson, invalidateQueries, queryKey, queryState, watchQuery } from "./query-cache.js";

/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */

const JOB_LIST = { commandId: "job.list", args: {}, invalidatedBy: ["job.state_changed"], subjectIds: [] };

/**
 * @param {any} result
 * @returns {ExecuteOutcome}
 */
function okOutcome(result) {
  return { ok: true, result, jobId: null, diagnostics: [], journalId: null };
}

/** Lets the mock host's answers (resolved promises) arrive. */
async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
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

describe("watchQuery", () => {
  test("fetches once, then again only on one of its own event types", async () => {
    const mock = countingHost();
    const stop = watchQuery(mock.host, JOB_LIST, () => {});
    await settle();
    expect(queryState(mock.host, queryKey(JOB_LIST))).toEqual({
      status: "ready",
      data: { jobs: [], answer: 1 },
      diagnostics: [],
    });

    mock.emit(sampleBusEvent("node.heartbeat", "node_01"));
    mock.emit(sampleBusEvent("run.metrics", "run_01"));
    await settle();
    expect(mock.calls).toHaveLength(1);

    mock.emit(sampleBusEvent("job.state_changed", "job_01"));
    await settle();
    expect(mock.calls).toHaveLength(2);
    expect(queryState(mock.host, queryKey(JOB_LIST)).data.answer).toBe(2);
    stop();
  });

  test("with subject ids, only events about those subjects refetch", async () => {
    const mock = createMockHost("core", { "job.get": () => okOutcome({ job: {} }) });
    const query = {
      commandId: "job.get",
      args: { job_id: "job_01" },
      invalidatedBy: ["job.state_changed"],
      subjectIds: ["job_01"],
    };
    const stop = watchQuery(mock.host, query, () => {});
    await settle();
    mock.emit(sampleBusEvent("job.state_changed", "job_02"));
    await settle();
    expect(mock.calls).toHaveLength(1);
    mock.emit(sampleBusEvent("job.state_changed", "job_01"));
    await settle();
    expect(mock.calls).toHaveLength(2);
    stop();
  });

  test("components asking for the same query share one entry and one request", async () => {
    const mock = countingHost();
    let changesSeenByFirst = 0;
    const stopFirst = watchQuery(mock.host, JOB_LIST, () => {
      changesSeenByFirst += 1;
    });
    const stopSecond = watchQuery(mock.host, { ...JOB_LIST, args: {} }, () => {});
    await settle();
    expect(mock.calls).toHaveLength(1);
    expect(changesSeenByFirst).toBe(1);
    stopFirst();
    stopSecond();
  });

  test("the last watcher to leave drops the entry and its event subscription", async () => {
    const mock = countingHost();
    const stop = watchQuery(mock.host, JOB_LIST, () => {});
    await settle();
    stop();
    expect(queryState(mock.host, queryKey(JOB_LIST)).status).toBe("loading");
    mock.emit(sampleBusEvent("job.state_changed", "job_01"));
    await settle();
    expect(mock.calls).toHaveLength(1);
  });

  test("a failed command is an error state with its diagnostics", async () => {
    const mock = createMockHost("core", {});
    const query = { commandId: "no.such_command", args: {}, invalidatedBy: [], subjectIds: [] };
    const stop = watchQuery(mock.host, query, () => {});
    await settle();
    const state = queryState(mock.host, queryKey(query));
    expect(state.status).toBe("error");
    expect(state.diagnostics.map((diagnostic) => diagnostic.code)).toEqual(["RRS-C0001"]);
    stop();
  });

  test("a host whose execute rejects leaves the entry in an error state, with no unhandled rejection", async () => {
    const mock = createMockHost("core", {});
    const rejecting = {
      ...mock.host,
      execute: async () => {
        throw new Error("the answer did not parse");
      },
    };
    const stop = watchQuery(rejecting, JOB_LIST, () => {});
    await settle();
    const state = queryState(rejecting, queryKey(JOB_LIST));
    expect(state.status).toBe("error");
    expect(state.data).toBeNull();
    expect(state.diagnostics).toHaveLength(1);
    expect(state.diagnostics[0].code).toBe("RRS-U0001");
    expect(state.diagnostics[0].message).toBe("the answer did not parse");
    stop();
  });

  test("an answer to an older request never replaces a newer one", async () => {
    /** @type {((outcome: ExecuteOutcome) => void)[]} */
    const pending = [];
    const mock = createMockHost("core", {});
    mock.host.execute = () => new Promise((resolve) => pending.push(resolve));
    const stop = watchQuery(mock.host, JOB_LIST, () => {});
    mock.emit(sampleBusEvent("job.state_changed", "job_01"));
    expect(pending).toHaveLength(2);
    pending[1](okOutcome({ answer: "newer" }));
    await settle();
    pending[0](okOutcome({ answer: "older" }));
    await settle();
    expect(queryState(mock.host, queryKey(JOB_LIST)).data).toEqual({ answer: "newer" });
    stop();
  });

  test("hosts never share results", async () => {
    const first = countingHost();
    const second = countingHost();
    const stopFirst = watchQuery(first.host, JOB_LIST, () => {});
    await settle();
    expect(queryState(second.host, queryKey(JOB_LIST)).status).toBe("loading");
    stopFirst();
  });
});

describe("invalidateQueries", () => {
  test("refetches every watched query of the host, as after a resync", async () => {
    const mock = countingHost();
    const stop = watchQuery(mock.host, JOB_LIST, () => {});
    await settle();
    invalidateQueries(mock.host);
    await settle();
    expect(mock.calls).toHaveLength(2);
    stop();
  });
});

describe("queryKey", () => {
  test("ignores the order of argument keys and of event types", () => {
    const first = queryKey({
      commandId: "job.list",
      args: { a: 1, b: { d: 2, c: 3 } },
      invalidatedBy: ["x", "y"],
      subjectIds: ["s1", "s2"],
    });
    const second = queryKey({
      commandId: "job.list",
      args: { b: { c: 3, d: 2 }, a: 1 },
      invalidatedBy: ["y", "x"],
      subjectIds: ["s2", "s1"],
    });
    expect(first).toBe(second);
  });

  test("tells different commands and arguments apart", () => {
    const base = { commandId: "job.get", args: { job_id: "job_01" }, invalidatedBy: [], subjectIds: [] };
    expect(queryKey(base)).not.toBe(queryKey({ ...base, args: { job_id: "job_02" } }));
    expect(queryKey(base)).not.toBe(queryKey({ ...base, commandId: "job.cancel" }));
    expect(queryKey(base)).not.toBe(queryKey({ ...base, subjectIds: ["job_01"] }));
  });

  test("canonicalJson sorts keys at every depth and keeps arrays in order", () => {
    expect(canonicalJson({ b: [3, 1], a: { z: 1, y: 2 } })).toBe('{"a":{"y":2,"z":1},"b":[3,1]}');
  });
});
