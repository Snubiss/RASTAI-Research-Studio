// The one event stream of the window (layout frontend-and-plugins.md 2.2, 8.4). Listeners register a filter; the hub
// subscribes the stream to the union of their event types plus the durable ones, reopens it with resume when that
// union changes (debounced), and pauses it while the window is hidden. The stream itself, its backoff and its resume
// are @rastai/client's openEventStream, passed in as openStream so tests can drive the stream by hand.

import { DURABLE_EVENT_TYPES, HIDDEN_PAUSE_MS, RESUBSCRIBE_DEBOUNCE_MS } from "../constants.js";

/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("@rastai/plugin").EventFilter} EventFilter */

/**
 * What the hub gives the stream as its handlers (@rastai/client's EventStreamHandlers).
 * @typedef {object} StreamHandlers
 * @property {(event: BusEvent) => void} onEvent
 * @property {(status: string, problem: Diagnostic | null) => void} onStatus
 * @property {() => void} onResync
 */

/**
 * One open stream, as openEventStream returns it: the hub stops it when the types change, and keeps the sequence itself.
 * @typedef {object} OpenedStream
 * @property {() => void} stop
 */

/**
 * @typedef {object} EventHubOptions
 * @property {(eventTypes: string[], afterBusSequence: bigint, handlers: StreamHandlers) => OpenedStream} openStream
 * @property {(status: string, problem: Diagnostic | null) => void} onStatus   the connection status, for the status bar
 * @property {() => void} onResync   the daemon no longer holds our gap (RRS-C0401): refetch the state
 */

/**
 * @typedef {object} Listener
 * @property {EventFilter} filter
 * @property {(event: BusEvent) => void} onEvent
 */

/**
 * @typedef {object} EventHub
 * @property {(filter: EventFilter, onEvent: (event: BusEvent) => void) => () => void} subscribe   returns unsubscribe
 * @property {() => void} start
 * @property {() => void} stop
 * @property {(hidden: boolean) => void} setHidden   a hidden window pauses its stream after HIDDEN_PAUSE_MS
 * @property {() => string[]} eventTypes            the types the stream is open for now
 * @property {() => boolean} isPaused
 */

/**
 * @param {EventHubOptions} options
 * @returns {EventHub}
 */
export function createEventHub(options) {
  /** @type {Listener[]} */
  let listeners = [];
  /** @type {OpenedStream | null} */
  let stream = null;
  /** @type {string[]} */
  let openedTypes = [];
  let lastSequence = 0n;
  let started = false;
  let paused = false;
  /** @type {number | null} */
  let reopenTimer = null;
  /** @type {number | null} */
  let hideTimer = null;

  /**
   * Opens the stream for the types the listeners need now, resuming after the last sequence it handed on.
   */
  function open() {
    if (stream !== null) {
      stream.stop();
    }
    openedTypes = wantedTypes();
    stream = options.openStream(openedTypes, lastSequence, {
      onEvent: handleEvent,
      onStatus: options.onStatus,
      onResync: options.onResync,
    });
  }

  /** Reopens the stream once the quiet time after a change has passed, if the union really changed. */
  function scheduleReopen() {
    if (!started || paused) {
      return;
    }
    if (reopenTimer !== null) {
      clearTimeout(reopenTimer);
    }
    reopenTimer = setTimeout(() => {
      reopenTimer = null;
      if (sameTypes(openedTypes, wantedTypes())) {
        return;
      }
      open();
    }, RESUBSCRIBE_DEBOUNCE_MS);
  }

  /** @param {BusEvent} event */
  function handleEvent(event) {
    lastSequence = event.busSequence;
    // A copy, so a listener that unsubscribes while it handles the event doesn't make the loop skip the next one.
    for (const listener of [...listeners]) {
      if (matches(listener.filter, event)) {
        listener.onEvent(event);
      }
    }
  }

  function pause() {
    if (stream !== null) {
      stream.stop();
      stream = null;
    }
    paused = true;
  }

  /** The types the stream needs: every listener's, or every type when one listener wants all of them. */
  function wantedTypes() {
    if (listeners.some((listener) => listener.filter.eventTypes.length === 0)) {
      return [];
    }
    const types = new Set(DURABLE_EVENT_TYPES);
    for (const listener of listeners) {
      listener.filter.eventTypes.forEach((type) => types.add(type));
    }
    return [...types].sort();
  }

  return {
    subscribe: (filter, onEvent) => {
      const listener = { filter, onEvent };
      listeners = [...listeners, listener];
      scheduleReopen();
      return () => {
        listeners = listeners.filter((other) => other !== listener);
        scheduleReopen();
      };
    },
    start: () => {
      started = true;
      open();
    },
    stop: () => {
      started = false;
      if (reopenTimer !== null) {
        clearTimeout(reopenTimer);
        reopenTimer = null;
      }
      if (hideTimer !== null) {
        clearTimeout(hideTimer);
        hideTimer = null;
      }
      if (stream !== null) {
        stream.stop();
        stream = null;
      }
    },
    setHidden: (hidden) => {
      if (hidden) {
        if (hideTimer === null && !paused && started) {
          hideTimer = setTimeout(() => {
            hideTimer = null;
            pause();
          }, HIDDEN_PAUSE_MS);
        }
        return;
      }
      if (hideTimer !== null) {
        clearTimeout(hideTimer);
        hideTimer = null;
      }
      if (paused && started) {
        paused = false;
        open();
      }
    },
    eventTypes: () => [...openedTypes],
    isPaused: () => paused,
  };
}

/**
 * Whether an event is one a listener asked for: its type is listed (an empty list is every type), and its subject is
 * listed or the filter names no subjects.
 * @param {EventFilter} filter
 * @param {BusEvent} event
 * @returns {boolean}
 */
export function matches(filter, event) {
  const typeWanted = filter.eventTypes.length === 0 || filter.eventTypes.includes(event.eventType);
  if (!typeWanted) {
    return false;
  }
  return filter.subjectIds.length === 0 || filter.subjectIds.includes(event.subjectId);
}

/**
 * @param {string[]} a
 * @param {string[]} b   both sorted
 * @returns {boolean}
 */
function sameTypes(a, b) {
  return a.length === b.length && a.every((type, index) => type === b[index]);
}
