// openEventStream: the one EventService.Subscribe stream of a window, with backoff, resume and the resync signal
// (layout frontend-and-plugins.md 2.2, 3.2; contracts 7.2). It owns its reconnect loop; only stop() ends it, or a
// refusal no retry can fix (a session, Host or Origin the gate refuses).

import { isGateRefusalError } from "./connection.js";
import {
  CODE_DAEMON_UNREACHABLE,
  CODE_EVENTS_MISSED,
  CODE_SIGNED_OUT,
  OFFLINE_AFTER_MS,
  RECONNECT_DELAYS_MS,
  STREAM_OPEN_GRACE_MS,
} from "./constants.js";
import { callFailureDiagnostic, clientDiagnostic, daemonDiagnosticOf } from "./diagnostics.js";

/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */

/** @typedef {"connecting" | "connected" | "reconnecting" | "resyncing" | "offline" | "signed_out"} EventStreamStatus */

/**
 * @typedef {object} EventStreamHandlers
 * @property {(event: BusEvent) => void} onEvent
 * @property {(status: EventStreamStatus, problem: Diagnostic | null) => void} onStatus
 * @property {() => void} onResync      the daemon no longer holds our gap (RRS-C0401): refetch state
 */

/**
 * @typedef {object} EventStream
 * @property {() => void} stop
 * @property {() => bigint} lastBusSequence
 */

/**
 * How one connection ended.
 * - "dropped": the connection broke, or the daemon closed it cleanly (it is stopping): reconnect.
 * - "gap_lost": RRS-C0401: resync, then start a new stream.
 * - "refused": the daemon refused the session or the request: stop.
 * - "stopped": stop() was called.
 * @typedef {object} StreamEnd
 * @property {"dropped" | "gap_lost" | "refused" | "stopped"} kind
 * @property {Diagnostic | null} problem
 */

/**
 * Everything one stream remembers between connections.
 * @typedef {object} StreamState
 * @property {bigint} lastSequence          the last bus_sequence handed to onEvent; 0n asks for a new stream
 * @property {boolean} stopped
 * @property {AbortController | null} connection   aborts the open connection on stop()
 * @property {(() => void) | null} wakeUp          ends the wait between attempts early, on stop()
 * @property {boolean} isOpen               the current connection counts as open (STREAM_OPEN_GRACE_MS)
 * @property {number} failedAttempts        connection attempts since the last one that opened
 * @property {number | null} failingSinceMs  when the stream last stopped working; null while it works
 * @property {boolean} resyncOnOpen         call onResync once the new stream is open, so nothing is missed
 */

// 0 asks the daemon for a new stream that starts with the next event (contracts 7.2): after RRS-C0401, and on
// every attempt until the stream has handed on its first event.
const NEW_STREAM_SEQUENCE = 0n;

/**
 * Opens the window's event stream and keeps it open until stop().
 * @param {DaemonClient} client
 * @param {string[]} eventTypes        empty for every event type
 * @param {bigint} afterBusSequence    the last sequence already seen; 0n for a new stream
 * @param {EventStreamHandlers} handlers
 * @returns {EventStream}
 */
export function openEventStream(client, eventTypes, afterBusSequence, handlers) {
  /** @type {StreamState} */
  const state = {
    lastSequence: afterBusSequence,
    stopped: false,
    connection: null,
    wakeUp: null,
    isOpen: false,
    failedAttempts: 0,
    failingSinceMs: null,
    resyncOnOpen: false,
  };
  runStream(client, eventTypes, state, handlers);
  return {
    stop: () => stopStream(state),
    lastBusSequence: () => state.lastSequence,
  };
}

/**
 * The reconnect loop: one connection at a time, until stop() or a refusal.
 * @param {DaemonClient} client
 * @param {string[]} eventTypes
 * @param {StreamState} state
 * @param {EventStreamHandlers} handlers
 */
async function runStream(client, eventTypes, state, handlers) {
  handlers.onStatus("connecting", null);
  while (!state.stopped) {
    const end = await readOneConnection(client, eventTypes, state, handlers);
    if (end.kind === "stopped") {
      return;
    }
    if (end.kind === "refused") {
      handlers.onStatus("signed_out", end.problem);
      return;
    }
    noteFailure(state);
    if (end.kind === "gap_lost") {
      state.lastSequence = NEW_STREAM_SEQUENCE;
      state.resyncOnOpen = true;
      handlers.onStatus("resyncing", end.problem);
    } else {
      noteMissedWhileDown(state);
      reportDrop(client, state, handlers, end.problem);
    }
    await waitBeforeReconnect(state);
  }
}

/**
 * Opens one Subscribe call and hands its events to onEvent until it ends.
 * @param {DaemonClient} client
 * @param {string[]} eventTypes
 * @param {StreamState} state
 * @param {EventStreamHandlers} handlers
 * @returns {Promise<StreamEnd>}
 */
async function readOneConnection(client, eventTypes, state, handlers) {
  const connection = new AbortController();
  state.connection = connection;
  state.isOpen = false;
  let cancelOpenTimer = () => {};
  const request = { afterBusSequence: state.lastSequence, eventTypes, subjectIds: [] };
  try {
    const events = client.events.subscribe(request, {
      signal: connection.signal,
      // The headers alone don't prove the daemon took the stream: it may end it right after them.
      onHeader: () => {
        cancelOpenTimer = markOpenAfterGrace(state, handlers);
      },
    });
    for await (const event of events) {
      markOpen(state, handlers);
      state.lastSequence = event.busSequence;
      handlers.onEvent(event);
    }
    return { kind: state.stopped ? "stopped" : "dropped", problem: null };
  } catch (error) {
    if (state.stopped) {
      return { kind: "stopped", problem: null };
    }
    return streamEndFromError(error, client.baseUrl);
  } finally {
    cancelOpenTimer();
    state.connection = null;
  }
}

/**
 * @param {any} error
 * @param {string} baseUrl
 * @returns {StreamEnd}
 */
function streamEndFromError(error, baseUrl) {
  const refusal = daemonDiagnosticOf(error);
  // The gate refuses a session (after a daemon restart, say) with HTTP 401, and a Host or Origin it doesn't allow
  // with 403, before any envelope, so the error carries no Diagnostic; only its Connect code tells it from a broken
  // connection, and no retry can fix it. Opening Rastai again (RRS-U0003's fix) gives a session at an allowed origin.
  if (refusal === null && isGateRefusalError(error)) {
    return { kind: "refused", problem: clientDiagnostic(CODE_SIGNED_OUT) };
  }
  if (refusal === null) {
    return { kind: "dropped", problem: clientDiagnostic(CODE_DAEMON_UNREACHABLE, { address: baseUrl }) };
  }
  if (refusal.code === CODE_EVENTS_MISSED) {
    return { kind: "gap_lost", problem: refusal };
  }
  // A session the daemon no longer knows becomes RRS-U0003; any other refusal is passed on as it came.
  return { kind: "refused", problem: callFailureDiagnostic(error, baseUrl) };
}

/**
 * Marks the stream open once it has stayed up STREAM_OPEN_GRACE_MS after its headers.
 * @param {StreamState} state
 * @param {EventStreamHandlers} handlers
 * @returns {() => void}   cancels the wait, when the connection ends first
 */
function markOpenAfterGrace(state, handlers) {
  const timer = setTimeout(() => markOpen(state, handlers), STREAM_OPEN_GRACE_MS);
  return () => clearTimeout(timer);
}

/**
 * The stream is open again: the backoff starts over, and a pending resync runs now, after the new stream is
 * subscribed, so an event that happens during the refetch still arrives. Runs once per connection.
 * @param {StreamState} state
 * @param {EventStreamHandlers} handlers
 */
function markOpen(state, handlers) {
  if (state.isOpen) {
    return;
  }
  state.isOpen = true;
  state.failedAttempts = 0;
  state.failingSinceMs = null;
  if (state.resyncOnOpen) {
    state.resyncOnOpen = false;
    handlers.onResync();
  }
  handlers.onStatus("connected", null);
}

/**
 * A dropped stream that has handed on no event has no sequence to resume from: it reconnects with 0, a new stream
 * from the next event, so whatever the daemon published while it was down is lost. onResync runs once it is open
 * again, so the app refetches what it missed.
 * @param {StreamState} state
 */
function noteMissedWhileDown(state) {
  if (state.lastSequence === NEW_STREAM_SEQUENCE) {
    state.resyncOnOpen = true;
  }
}

/** @param {StreamState} state */
function noteFailure(state) {
  state.failedAttempts += 1;
  if (state.failingSinceMs === null) {
    state.failingSinceMs = Date.now();
  }
}

/**
 * "reconnecting" at first; "offline" once the stream has not worked for OFFLINE_AFTER_MS.
 * @param {DaemonClient} client
 * @param {StreamState} state
 * @param {EventStreamHandlers} handlers
 * @param {Diagnostic | null} problem
 */
function reportDrop(client, state, handlers, problem) {
  const failingForMs = Date.now() - (state.failingSinceMs ?? Date.now());
  if (failingForMs >= OFFLINE_AFTER_MS) {
    handlers.onStatus("offline", clientDiagnostic(CODE_DAEMON_UNREACHABLE, { address: client.baseUrl }));
  } else {
    handlers.onStatus("reconnecting", problem);
  }
}

/**
 * The delay before the next attempt: RECONNECT_DELAYS_MS in order, then its last value for good.
 * @param {number} failedAttempts   at least 1
 * @returns {number}
 */
export function reconnectDelayMs(failedAttempts) {
  const index = Math.min(failedAttempts, RECONNECT_DELAYS_MS.length) - 1;
  return RECONNECT_DELAYS_MS[index];
}

/**
 * Waits out the backoff; stop() ends the wait at once.
 * @param {StreamState} state
 * @returns {Promise<void>}
 */
function waitBeforeReconnect(state) {
  return new Promise((resolve) => {
    const timer = setTimeout(finish, reconnectDelayMs(state.failedAttempts));
    function finish() {
      clearTimeout(timer);
      state.wakeUp = null;
      resolve();
    }
    state.wakeUp = finish;
  });
}

/** @param {StreamState} state */
function stopStream(state) {
  state.stopped = true;
  state.connection?.abort();
  state.wakeUp?.();
}
