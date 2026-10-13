// Whether a bus event is one a subscriber asked for (layout frontend-and-plugins.md 7.2): its type is listed, and
// its subject is listed or the filter names no subjects. The daemon filters the window's one stream the same way.

/** @typedef {import("./host-api.js").EventFilter} EventFilter */
/** @typedef {import("./host-api.js").BusEvent} BusEvent */

/**
 * @param {EventFilter} filter
 * @param {BusEvent} event
 * @returns {boolean}
 */
export function eventMatches(filter, event) {
  if (!filter.eventTypes.includes(event.eventType)) {
    return false;
  }
  return filter.subjectIds.length === 0 || filter.subjectIds.includes(event.subjectId);
}
