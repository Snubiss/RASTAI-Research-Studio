// The status bar (design 3.5): the connection, the node's telemetry, the running jobs, and the locked egress badge on
// the right. The badge shows only while data leaves the machine, and the setting privacy.show_egress_badge hides it.
// Each figure comes from the daemon's events and commands; the bar only formats them.

import { EgressBadge } from "@rastai/ui/security";
import { useStore } from "@rastai/plugin";
import { useEffect, useState } from "react";

import { ACTIVE_JOB_STATES } from "../constants.js";
import { useShell } from "../app/shell-context.js";
import { readSetting } from "../state/settings.js";
import { activeEgress } from "./egress-state.js";

const BYTES_PER_GIB = 1024 * 1024 * 1024;
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
  const jobs = /** @type {{ state: string }[]} */ (useStore(shell.stores.jobs));
  const egress = useStore(shell.stores.egress);
  const now = useClock(egress.length > 0);
  const badgeOn = readSetting(settings.values, "privacy.show_egress_badge", true) === true;
  const held = activeEgress(egress, now);
  const running = jobs.filter((job) => ACTIVE_JOB_STATES.includes(job.state)).length;
  const node = nodes[0] ?? null;
  const connectionWord = CONNECTION_WORDS[connection.status] ?? connection.status;
  return (
    <footer id="status-bar" className="rr-status-bar" aria-label="Status bar" tabIndex={-1}>
      <span className="rr-status-bar__connection">
        <span aria-hidden="true" className={`rr-status-bar__dot rr-status-bar__dot--${connection.status}`} />
        {connectionWord}
      </span>
      <span className="rr-status-bar__node">{nodeSummary(node, node === null ? null : telemetry[node.node_id])}</span>
      <span className="rr-status-bar__jobs">{running === 0 ? "No jobs running" : `${running} running`}</span>
      <span className="rr-status-bar__spacer" />
      {badgeOn && held.length > 0 ? (
        <EgressBadge activeEgress={held} onOpen={() => shell.stores.overlay.set("egress")} />
      ) : null}
      <span className="rr-status-bar__version">{shell.client.daemonVersion}</span>
    </footer>
  );
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
