// Server state for useQuery (layout frontend-and-plugins.md 2.3): command results as they came, never computed in
// the browser. One cache per host, so two plugins never share a result one of them may not read. Components that
// show the same command with the same arguments and events share one entry and one request. An entry lives while a
// component watches it; it refetches when one of its events arrives and is dropped when the last watcher leaves,
// so a result is never served after events it would have missed.

/** @typedef {import("../host-api.js").PluginHost} PluginHost */
/** @typedef {import("../host-api.js").Diagnostic} Diagnostic */
/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */

import { callFailedDiagnostic, refusedOutcome } from "../plugin-diagnostic.js";

/**
 * @typedef {object} QueryState
 * @property {"loading" | "ready" | "error"} status
 * @property {any} data                       the parsed result_json; null until ready
 * @property {Diagnostic[]} diagnostics
 */

/**
 * What a component asks for.
 * @typedef {object} Query
 * @property {string} commandId
 * @property {object} args
 * @property {string[]} invalidatedBy   event types, e.g. ["conversation.changed"]
 * @property {string[]} subjectIds      only events about these subjects invalidate it; empty: any subject
 */

/**
 * @typedef {object} QueryEntry
 * @property {Query} query
 * @property {QueryState} state
 * @property {Set<() => void>} watchers
 * @property {number} latestFetch          number of the newest request; an older answer is ignored
 * @property {() => void} stopEvents
 */

/** @type {QueryState} */
export const LOADING_STATE = { status: "loading", data: null, diagnostics: [] };

/** @type {WeakMap<PluginHost, Map<string, QueryEntry>>} */
const cachesByHost = new WeakMap();

/**
 * The cache key: the command, its arguments as canonical JSON (keys sorted, so { a, b } and { b, a } share an
 * entry) and the events that invalidate it.
 * @param {Query} query
 * @returns {string}
 */
export function queryKey(query) {
  const events = [[...query.invalidatedBy].sort(), [...query.subjectIds].sort()];
  return JSON.stringify([query.commandId, canonicalJson(query.args), events]);
}

/**
 * JSON with every object's keys in sorted order.
 * @param {any} value
 * @returns {string}
 */
export function canonicalJson(value) {
  return JSON.stringify(value, (_key, member) => {
    if (member === null || typeof member !== "object" || Array.isArray(member)) {
      return member;
    }
    /** @type {Object<string, any>} */
    const sorted = {};
    for (const name of Object.keys(member).sort()) {
      sorted[name] = member[name];
    }
    return sorted;
  });
}

/**
 * The state a component shows for a query: the entry's, or loading before anyone watches it.
 * @param {PluginHost} host
 * @param {string} key
 * @returns {QueryState}
 */
export function queryState(host, key) {
  return cacheOf(host).get(key)?.state ?? LOADING_STATE;
}

/**
 * Starts watching a query: the first watcher subscribes to its events and fetches it.
 * @param {PluginHost} host
 * @param {Query} query
 * @param {() => void} onChange   called whenever the entry's state changes
 * @returns {() => void} stops watching; the last watcher to stop drops the entry
 */
export function watchQuery(host, query, onChange) {
  const cache = cacheOf(host);
  const key = queryKey(query);
  let entry = cache.get(key);
  if (!entry) {
    entry = openEntry(host, query);
    cache.set(key, entry);
  }
  entry.watchers.add(onChange);
  const watched = entry;
  return () => {
    watched.watchers.delete(onChange);
    if (watched.watchers.size === 0 && cache.get(key) === watched) {
      watched.stopEvents();
      cache.delete(key);
    }
  };
}

/**
 * Refetches every query this host's components watch. The app calls it after a resync (RRS-C0401, layout 2.2):
 * events were lost, so any result may be stale.
 * @param {PluginHost} host
 */
export function invalidateQueries(host) {
  for (const entry of cacheOf(host).values()) {
    fetchEntry(host, entry);
  }
}

/**
 * A new entry, subscribed to its events and fetching its first result.
 * @param {PluginHost} host
 * @param {Query} query
 * @returns {QueryEntry}
 */
function openEntry(host, query) {
  /** @type {QueryEntry} */
  const entry = { query, state: LOADING_STATE, watchers: new Set(), latestFetch: 0, stopEvents: () => {} };
  if (query.invalidatedBy.length > 0) {
    const filter = { eventTypes: query.invalidatedBy, subjectIds: query.subjectIds };
    entry.stopEvents = host.subscribe(filter, () => fetchEntry(host, entry));
  }
  fetchEntry(host, entry);
  return entry;
}

/**
 * Runs the command and stores what it returned. A refetch keeps showing the last result until the new one arrives,
 * so a list doesn't flash back to loading on every event.
 * @param {PluginHost} host
 * @param {QueryEntry} entry
 */
async function fetchEntry(host, entry) {
  entry.latestFetch += 1;
  const thisFetch = entry.latestFetch;
  const outcome = await executeQuery(host, entry.query);
  if (thisFetch !== entry.latestFetch) {
    return; // a newer request was sent after this one; its answer wins
  }
  if (outcome.ok) {
    entry.state = { status: "ready", data: outcome.result, diagnostics: outcome.diagnostics };
  } else {
    entry.state = { status: "error", data: null, diagnostics: outcome.diagnostics };
  }
  entry.watchers.forEach((watcher) => watcher());
}

/**
 * Runs the query's command. A host whose execute rejects still gives the entry an answer: an error outcome, so the
 * entry leaves loading, and nothing is left as an unhandled rejection.
 * @param {PluginHost} host
 * @param {Query} query
 * @returns {Promise<ExecuteOutcome>}
 */
async function executeQuery(host, query) {
  try {
    return await host.execute(query.commandId, query.args);
  } catch (error) {
    return refusedOutcome(callFailedDiagnostic(error));
  }
}

/**
 * @param {PluginHost} host
 * @returns {Map<string, QueryEntry>}
 */
function cacheOf(host) {
  let cache = cachesByHost.get(host);
  if (!cache) {
    cache = new Map();
    cachesByHost.set(host, cache);
  }
  return cache;
}
