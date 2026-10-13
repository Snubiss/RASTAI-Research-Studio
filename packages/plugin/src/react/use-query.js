// Server state in a component (layout frontend-and-plugins.md 2.3): runs a read command and runs it again when one
// of the named bus events arrives. The results live in state/query-cache.js, one cache per host.

import { useCallback, useSyncExternalStore } from "react";

import { queryKey, queryState, watchQuery } from "../state/query-cache.js";
import { usePluginHost } from "./use-plugin-host.js";

/** @typedef {import("../state/query-cache.js").QueryState} QueryState */

/** @type {string[]} */
const NO_SUBJECTS = [];

/**
 * Runs a read command and re-runs it when one of the named events arrives.
 * @param {string} commandId
 * @param {object} args
 * @param {string[]} invalidatedBy   event types, e.g. ["conversation.changed"]
 * @param {string[]} [subjectIds]    only events about these subjects, e.g. [jobId]; empty or left out: any subject
 * @returns {QueryState}
 */
export function useQuery(commandId, args, invalidatedBy, subjectIds = NO_SUBJECTS) {
  const host = usePluginHost();
  const query = { commandId, args, invalidatedBy, subjectIds };
  // The key is a string, so a new args object with the same content each render doesn't restart the query.
  const key = queryKey(query);
  const subscribe = useCallback(
    /** @param {() => void} onChange */
    (onChange) => watchQuery(host, query, onChange),
    // `query` is left out on purpose: `key` describes all of it, and a new object each render must not resubscribe.
    [host, key],
  );
  return useSyncExternalStore(subscribe, () => queryState(host, key));
}
