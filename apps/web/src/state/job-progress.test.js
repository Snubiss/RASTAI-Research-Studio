// The job progress map: which job a job.progress event is about, and when a job's progress leaves the map.

import { JobState } from "@rastai/proto";
import { describe, expect, test } from "vitest";

import { jobIdOfEvent, withJobProgress, withOnlyListedJobs, withoutEndedJob } from "./job-progress.js";

const RUN_ID = "run_01";
const JOB_ID = "job_01";

/**
 * A job.progress event as the daemon sends it: the subject and the source the test sets.
 * @param {string} source
 * @param {string} subjectId
 * @returns {any}
 */
function progressEvent(source, subjectId) {
  const payload = { payload: { case: "jobProgress", value: { fraction: 0.25, message: "tokenizing" } } };
  return { busSequence: 1n, eventType: "job.progress", subjectId, source, payload };
}

/**
 * The progress map of one job half done.
 * @returns {any}
 */
function halfDone() {
  return { [JOB_ID]: { fraction: 0.5, message: "" } };
}

/**
 * A job.state_changed event that moves a job to a state.
 * @param {string} jobId
 * @param {number} toState
 * @returns {any}
 */
function stateEvent(jobId, toState) {
  const value = { jobId, fromState: JobState.RUNNING, toState };
  return {
    busSequence: 2n,
    eventType: "job.state_changed",
    subjectId: jobId,
    source: "daemon",
    payload: { payload: { case: "jobStateChanged", value } },
  };
}

describe("which job a progress event is about", () => {
  test("the job id in the source wins over a run id as the subject", () => {
    expect(jobIdOfEvent(progressEvent(`job:${JOB_ID}`, RUN_ID))).toBe(JOB_ID);
  });

  test("with no job source the subject is the job", () => {
    expect(jobIdOfEvent(progressEvent("daemon", JOB_ID))).toBe(JOB_ID);
  });

  test("an empty job source falls back to the subject", () => {
    expect(jobIdOfEvent(progressEvent("job:", JOB_ID))).toBe(JOB_ID);
  });
});

describe("the progress map", () => {
  test("a run-subject progress event is keyed by its job, not by the run", () => {
    const progress = withJobProgress({}, progressEvent(`job:${JOB_ID}`, RUN_ID));
    expect(Object.keys(progress)).toEqual([JOB_ID]);
    expect(progress[JOB_ID].fraction).toBe(0.25);
  });

  test("a job that reaches an ended state loses its progress", () => {
    const running = withJobProgress({}, progressEvent(`job:${JOB_ID}`, RUN_ID));
    const ended = withoutEndedJob(running, stateEvent(JOB_ID, JobState.SUCCEEDED));
    expect(ended).toEqual({});
  });

  test("failed and cancelled end a job too", () => {
    const running = halfDone();
    expect(withoutEndedJob(running, stateEvent(JOB_ID, JobState.FAILED))).toEqual({});
    expect(withoutEndedJob(running, stateEvent(JOB_ID, JobState.CANCELLED))).toEqual({});
  });

  test("a state change that is not final keeps the progress", () => {
    const running = halfDone();
    expect(withoutEndedJob(running, stateEvent(JOB_ID, JobState.PAUSED))).toBe(running);
  });

  test("an ended job with no progress leaves the map as it was", () => {
    const running = { other: halfDone()[JOB_ID] };
    expect(withoutEndedJob(running, stateEvent(JOB_ID, JobState.SUCCEEDED))).toBe(running);
  });
});

describe("the progress map after the job list is refreshed", () => {
  test("a job the list no longer holds loses its progress", () => {
    /** @type {any} */
    const progress = { [JOB_ID]: { fraction: 0.5, message: "" }, job_02: { fraction: 0.1, message: "" } };
    const kept = withOnlyListedJobs(progress, [{ job_id: "job_02" }]);
    expect(Object.keys(kept)).toEqual(["job_02"]);
  });

  test("a job the list still holds keeps its progress", () => {
    const progress = halfDone();
    expect(withOnlyListedJobs(progress, [{ job_id: JOB_ID }])).toEqual(progress);
  });
});
