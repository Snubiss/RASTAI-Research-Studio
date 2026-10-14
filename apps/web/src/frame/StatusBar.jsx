// The status bar (design 3.5): the connection, the node's telemetry, the running jobs with their progress, and the
// locked egress badge on the right. The badge shows only while data leaves the machine, and the setting
// privacy.show_egress_badge hides it. Each figure comes from the daemon's events and commands; the bar only formats them.
// A running job is a button that opens its run's page. The bar lists the first few and says how many more run.

import { EgressBadge } from "@rastai/ui/security";
import { useStore } from "@rastai/plugin";
import { useEffect, useState } from "react";

import { ACTIVE_JOB_STATES, HOME_SECTION_ID, JOB_KIND_SECTIONS } from "../constants.js";
import { useShell } from "../app/shell-context.js";
import { navigateTo } from "../routing/navigate.js";
import { sectionRoute } from "../routing/route.js";
import { readSetting } from "../state/settings.js";
import { activeEgress } from "./egress-state.js";

/** @typedef {import("@rastai/formats").JobSummary} JobSummary */
/** @typedef {import("../routing/route.js").Route} Route */
/** @typedef {import("@rastai/proto").JobProgress} JobProgress */

const BYTES_PER_GIB = 1024 * 1024 * 1024;
// The running jobs the bar lists by name; the rest are counted ("2 more running"), so a busy node can't fill the bar.
const STATUS_BAR_JOB_LIMIT = 3;
// The route segment that opens a run's page (layout frontend-and-plugins.md 2.6): #/<section>/run/<run_id>.
const RUN_ROUTE_SEGMENT = "run";
// The egress list is re-checked this often while it has entries, so a held entry drops off after its hold time.
const EGRESS_RECHECK_MS = 1000;
/** @type {Object<string, string>} */
const CONNECTION_WORDS = {
  connecting: "Connecting…",
  connected: "Connected",
  reconnecting: "Reconnecting…",
  resyncing: "Resyncing…",
  offline: "Offline",
  signed_out: "Signed out",
};

export function StatusBar() {
  const shell = useShell();
  const connection = useStore(shell.stores.connection);
  const settings = useStore(shell.stores.settings);
  const nodes = useStore(shell.stores.nodes);
  const telemetry = useStore(shell.stores.telemetry);
  const jobs = /** @type {JobSummary[]} */ (useStore(shell.stores.jobs));
  const jobProgress = useStore(shell.stores.jobProgress);
  const egress = useStore(shell.stores.egress);
  const now = useClock(egress.length > 0);
  const badgeOn = readSetting(settings.values, "privacy.show_egress_badge", true) === true;
  const held = activeEgress(egress, now);
  const runningJobs = jobs.filter((job) => ACTIVE_JOB_STATES.includes(job.state));
  const listedJobs = runningJobs.slice(0, STATUS_BAR_JOB_LIMIT);
  const unlistedCount = runningJobs.length - listedJobs.length;
  const node = nodes[0] ?? null;
  const connectionWord = CONNECTION_WORDS[connection.status] ?? connection.status;
  return (
    <footer id="status-bar" className="rr-status-bar" aria-label="Status bar" tabIndex={-1}>
      <span className="rr-status-bar__connection">
        <span aria-hidden="true" className={`rr-status-bar__dot rr-status-bar__dot--${connection.status}`} />
        {connectionWord}
      </span>
      <span className="rr-status-bar__node">{nodeSummary(node, node === null ? null : telemetry[node.node_id])}</span>
      <span className="rr-status-bar__jobs">
        {runningJobs.length === 0 ? "No jobs running" : `${runningJobs.length} running`}
      </span>
      {listedJobs.map((job) => (
        <RunningJob key={job.job_id} job={job} progress={jobProgress[job.job_id] ?? null} />
      ))}
      {unlistedCount > 0 ? <span className="rr-status-bar__more">{`${unlistedCount} more running`}</span> : null}
      <span className="rr-status-bar__spacer" />
      {badgeOn && held.length > 0 ? (
        <EgressBadge activeEgress={held} onOpen={() => shell.stores.overlay.set("egress")} />
      ) : null}
      <span className="rr-status-bar__version">{shell.client.daemonVersion}</span>
    </footer>
  );
}

/**
 * One running job: a button that opens it, with its name and, once the daemon has sent progress for it, a mini bar.
 * @param {object} props
 * @param {JobSummary} props.job
 * @param {JobProgress | null} props.progress
 */
function RunningJob({ job, progress }) {
  const name = jobName(job);
  return (
    <button type="button" className="rr-status-bar__job" onClick={() => navigateTo(jobRouteOf(job))}>
      <span>{name}</span>
      {progress === null ? null : <progress max={1} value={progress.fraction} aria-label={`${name} progress`} />}
    </button>
  );
}

/**
 * The page a running job opens: its run's page in the job kind's section (the training section for a train job).
 * No job page exists yet, so a job without a run opens its section's page, and a kind with no section opens Home.
 * @param {JobSummary} job
 * @returns {Route}
 */
export function jobRouteOf(job) {
  const sectionId = JOB_KIND_SECTIONS[job.kind];
  if (sectionId === undefined) {
    return sectionRoute(HOME_SECTION_ID);
  }
  return { sectionId, path: job.run_id ? [RUN_ROUTE_SEGMENT, job.run_id] : [], query: {} };
}

/**
 * A job has no name of its own (JobSummary), so the bar names it by its kind, with its run when it has one.
 * @param {JobSummary} job
 * @returns {string}
 */
export function jobName(job) {
  return job.run_id ? `${job.kind} · ${job.run_id}` : job.kind;
}

/**
 * The time, re-read every second while something is on the egress list, so the badge drops off after its hold time.
 * @param {boolean} ticking
 * @returns {number}   the time of the last re-read
 */
function useClock(ticking) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) {
      return undefined;
    }
    const timer = setInterval(() => setNow(Date.now()), EGRESS_RECHECK_MS);
    return () => clearInterval(timer);
  }, [ticking]);
  return now;
}

/**
 * "local · GPU 54% · VRAM 4.6 / 16 GiB" from the node and its last telemetry. Without a sample it says so.
 * @param {{ name: string, node_id: string } | null} node
 * @param {any} sample   the node's last NodeTelemetry, or null
 * @returns {string}
 */
export function nodeSummary(node, sample) {
  if (node === null) {
    return "No node";
  }
  if (sample === null || sample === undefined || sample.gpus.length === 0) {
    return `${node.name}: no GPU telemetry yet`;
  }
  const gpu = sample.gpus[0];
  const used = gibibytes(gpu.vramUsedBytes);
  const total = gibibytes(gpu.vramTotalBytes);
  return `${node.name} · GPU ${gpu.utilizationPercent}% · VRAM ${used} / ${total} GiB`;
}

/**
 * @param {bigint | number} bytes
 * @returns {string}   one decimal place
 */
function gibibytes(bytes) {
  return (Number(bytes) / BYTES_PER_GIB).toFixed(1);
}
