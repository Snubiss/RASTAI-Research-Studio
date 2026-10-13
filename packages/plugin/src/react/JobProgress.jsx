// The state and progress of any job, with its final Diagnostic (layout frontend-and-plugins.md 7.1). The state is
// job.get's, read again on each of the job's own job.state_changed events; the fraction and message come from the ephemeral job.progress
// events, which may be dropped under load, so they only ever move the bar and are never the record.

import { useEffect, useState } from "react";

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
  return (
    <div aria-busy={job.status === "loading"}>
      {state ? <span>{state}</span> : null}
      {events.fraction !== null ? <progress max={1} value={events.fraction} aria-label="Job progress" /> : null}
      {events.message ? <span>{events.message}</span> : null}
      <DiagnosticList diagnostics={job.diagnostics} />
      {events.diagnostic ? <DiagnosticList diagnostics={[events.diagnostic]} /> : null}
    </div>
  );
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
