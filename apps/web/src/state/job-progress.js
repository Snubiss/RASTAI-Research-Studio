// The job progress the status bar draws (design 3.5). A job.progress event names its job two ways: a job's own events
// carry `job:<job_id>` as their source, while the subject is the run id for run-log records (contracts 7.2). The bar
// keys progress by the job id the job list uses, so the source is read first. Pure functions only: the shell stores
// what they return.

import { JobState } from "@rastai/proto";

/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("@rastai/proto").JobProgress} JobProgress */
/** @typedef {Object<string, JobProgress>} JobProgressMap */

const JOB_SOURCE_PREFIX = "job:";
// The states a job ends in (contracts 9.1). A job in any other state can still send progress, so it stays on the bar.
const ENDED_JOB_STATES = [JobState.SUCCEEDED, JobState.FAILED, JobState.CANCELLED];

/**
 * The job a job.progress event is about: the job id in its source, or the subject when the source names no job.
 * @param {BusEvent} event
 * @returns {string}
 */
export function jobIdOfEvent(event) {
  if (event.source.startsWith(JOB_SOURCE_PREFIX)) {
    const jobId = event.source.slice(JOB_SOURCE_PREFIX.length);
    if (jobId !== "") {
      return jobId;
    }
  }
  return event.subjectId;
}

/**
 * The map with the event's progress set for its job. An event that carries no progress leaves the map as it was.
 * @param {JobProgressMap} progressByJob
 * @param {BusEvent} event
 * @returns {JobProgressMap}
 */
export function withJobProgress(progressByJob, event) {
  // The bus event's payload is an EventPayload, whose oneof names the progress (contracts 4.3).
  const payload = /** @type {any} */ (event.payload)?.payload;
  if (payload?.case !== "jobProgress") {
    return progressByJob;
  }
  return { ...progressByJob, [jobIdOfEvent(event)]: payload.value };
}

/**
 * The map without the job whose job.state_changed event moved it to an ended state. Any other event leaves it as it was.
 * @param {JobProgressMap} progressByJob
 * @param {BusEvent} event
 * @returns {JobProgressMap}
 */
export function withoutEndedJob(progressByJob, event) {
  const payload = /** @type {any} */ (event.payload)?.payload;
  if (payload?.case !== "jobStateChanged" || !ENDED_JOB_STATES.includes(payload.value.toState)) {
    return progressByJob;
  }
  const jobId = payload.value.jobId;
  if (!(jobId in progressByJob)) {
    return progressByJob;
  }
  const remaining = { ...progressByJob };
  delete remaining[jobId];
  return remaining;
}

/**
 * The map without the progress of any job the job list no longer holds. A resync (RRS-C0401) can miss a job's final
 * event, so the refreshed list is what tells the bar the job is gone; without it the entry would stay until reload.
 * @param {JobProgressMap} progressByJob
 * @param {{ job_id: string }[]} listedJobs
 * @returns {JobProgressMap}
 */
export function withOnlyListedJobs(progressByJob, listedJobs) {
  const listedIds = new Set(listedJobs.map((job) => job.job_id));
  /** @type {JobProgressMap} */
  const kept = {};
  for (const jobId of Object.keys(progressByJob)) {
    if (listedIds.has(jobId)) {
      kept[jobId] = progressByJob[jobId];
    }
  }
  return kept;
}
