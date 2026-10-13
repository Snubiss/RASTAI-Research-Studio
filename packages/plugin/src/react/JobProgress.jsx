// The state and progress of any job, with its final Diagnostic (layout frontend-and-plugins.md 7.1). The state is
// job.get's, read again on each of the job's own job.state_changed events; the fraction and message come from the
// ephemeral job.progress events, which may be dropped under load, so they only ever move the bar and are never the
// record. The final Diagnostic is the state change's when this view saw it happen, and otherwise job.get's
// error_code, so a job that failed before the view opened (after a reload or a section switch) still says why.

import { useEffect, useState } from "react";

import { pluginDiagnostic } from "../plugin-diagnostic.js";
import { DiagnosticList } from "./DiagnosticList.jsx";
import { usePluginHost } from "./use-plugin-host.js";
import { useQuery } from "./use-query.js";

/** @typedef {import("../host-api.js").BusEvent} BusEvent */
/** @typedef {import("../host-api.js").Diagnostic} Diagnostic */

const JOB_STATE_CHANGED = "job.state_changed";
const JOB_PROGRESS = "job.progress";

/**
 * What the job's events have said so far.
 * @typedef {object} JobEvents
 * @property {number | null} fraction        0 to 1; null before the first progress event
 * @property {string} message
 * @property {Diagnostic | null} diagnostic  the last state change's, when it carried one
 */

/** @type {JobEvents} */
const NO_JOB_EVENTS = { fraction: null, message: "", diagnostic: null };

/**
 * @param {object} props
 * @param {string} props.jobId
 */
export function JobProgress({ jobId }) {
  const job = useQuery("job.get", { job_id: jobId }, [JOB_STATE_CHANGED], [jobId]);
  const events = useJobEvents(jobId);
  const state = job.status === "ready" ? job.data.job.state : "";
  const errorCode = job.status === "ready" ? (job.data.job.error_code ?? "") : "";
  const finalDiagnostic = events.diagnostic ?? storedFailure(errorCode, jobId);
  return (
    <div aria-busy={job.status === "loading"}>
      {state ? <span>{state}</span> : null}
      {events.fraction !== null ? <progress max={1} value={events.fraction} aria-label="Job progress" /> : null}
      {events.message ? <span>{events.message}</span> : null}
      <DiagnosticList diagnostics={job.diagnostics} />
      {finalDiagnostic ? <DiagnosticList diagnostics={[finalDiagnostic]} /> : null}
    </div>
  );
}

/**
 * The Diagnostic for the code a job failed with, as job.get stores it; null for a job that hasn't failed. Only the
 * code is stored, so the message's other placeholders stay unfilled and the catalog's plain text and fix lead.
 * @param {string} errorCode
 * @param {string} jobId
 * @returns {Diagnostic | null}
 */
function storedFailure(errorCode, jobId) {
  if (errorCode === "") {
    return null;
  }
  return pluginDiagnostic(errorCode, { job_id: jobId });
}

/**
 * Follows the job's progress and state-change events.
 * @param {string} jobId
 * @returns {JobEvents}
 */
function useJobEvents(jobId) {
  const host = usePluginHost();
  const [events, setEvents] = useState(NO_JOB_EVENTS);
  useEffect(() => {
    setEvents(NO_JOB_EVENTS);
    const filter = { eventTypes: [JOB_PROGRESS, JOB_STATE_CHANGED], subjectIds: [jobId] };
    return host.subscribe(filter, (event) => setEvents((previous) => withEvent(previous, event)));
  }, [host, jobId]);
  return events;
}

/**
 * @param {JobEvents} previous
 * @param {BusEvent} event
 * @returns {JobEvents}
 */
function withEvent(previous, event) {
  const payload = event.payload?.payload;
  if (payload?.case === "jobProgress") {
    return { ...previous, fraction: payload.value.fraction, message: payload.value.message };
  }
  if (payload?.case === "jobStateChanged") {
    const diagnostic = payload.value.diagnostic;
    return { ...previous, diagnostic: diagnostic && diagnostic.code !== "" ? diagnostic : previous.diagnostic };
  }
  return previous;
}
