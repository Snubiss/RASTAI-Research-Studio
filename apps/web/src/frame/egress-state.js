// What the egress badge shows (spec 24; layout frontend-and-plugins.md 8.4): the active {purpose, host, subject_id}
// pairs from egress.active events only, each held for EGRESS_BADGE_HOLD_MS after its last event, so a transfer that
// reports every few seconds never flickers. Pure functions over a list of entries; the status bar gives them the time.

import { EGRESS_BADGE_HOLD_MS } from "../constants.js";

/** @typedef {import("@rastai/ui/security").ActiveEgress} ActiveEgress */

/**
 * One reported egress, with the time it was last reported.
 * @typedef {object} EgressEntry
 * @property {string} purpose
 * @property {string} host
 * @property {string} subjectId
 * @property {number} lastSeenUnixMs
 */

/**
 * The entries with one report added or refreshed. A pair (purpose, host, subject) is one entry.
 * @param {EgressEntry[]} entries
 * @param {ActiveEgress} report
 * @param {number} nowUnixMs
 * @returns {EgressEntry[]}
 */
export function recordEgress(entries, report, nowUnixMs) {
  const others = entries.filter((entry) => !sameEgress(entry, report));
  return [...others, { ...report, lastSeenUnixMs: nowUnixMs }];
}

/**
 * The entries still held at this time; an entry older than the hold time is dropped.
 * @param {EgressEntry[]} entries
 * @param {number} nowUnixMs
 * @returns {EgressEntry[]}
 */
export function heldEgress(entries, nowUnixMs) {
  return entries.filter((entry) => nowUnixMs - entry.lastSeenUnixMs < EGRESS_BADGE_HOLD_MS);
}

/**
 * What the badge lists: the held entries, as the locked badge reads them.
 * @param {EgressEntry[]} entries
 * @param {number} nowUnixMs
 * @returns {ActiveEgress[]}
 */
export function activeEgress(entries, nowUnixMs) {
  return heldEgress(entries, nowUnixMs).map((entry) => ({
    purpose: entry.purpose,
    host: entry.host,
    subjectId: entry.subjectId,
  }));
}

/**
 * The egress report inside a bus event, or null: only egress.active carries one. Its payload arrives with M2
 * (contracts 7.3); until then no event has it and the badge stays off.
 * @param {any} event   a BusEvent
 * @returns {ActiveEgress | null}
 */
export function egressReportOf(event) {
  // The payload is an EventPayload; its oneof (the `payload` field) names the report.
  const oneof = event?.payload?.payload;
  if (event?.eventType !== "egress.active" || oneof?.case !== "egressActive") {
    return null;
  }
  const value = oneof.value;
  return {
    purpose: String(value.purpose ?? ""),
    host: String(value.host ?? ""),
    subjectId: String(value.subjectId ?? ""),
  };
}

/**
 * @param {EgressEntry} entry
 * @param {ActiveEgress} report
 * @returns {boolean}
 */
function sameEgress(entry, report) {
  return entry.purpose === report.purpose && entry.host === report.host && entry.subjectId === report.subjectId;
}
