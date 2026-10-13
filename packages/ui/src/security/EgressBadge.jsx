// "Data is leaving this machine" (owner decision N7; layout frontend-and-plugins.md 2.10). The app shell renders it
// in the status bar, outside every panel; it shows only while something is sending data out, and a click opens the
// details. Drawn only with the locked egress colors.

import { BADGE_STYLE } from "./locked-styles.js";

/**
 * One active outbound transfer: why, to where, and for which job, run or model.
 * @typedef {object} ActiveEgress
 * @property {string} purpose     e.g. "model download"
 * @property {string} host        e.g. "huggingface.co"
 * @property {string} subjectId
 */

/**
 * @param {object} props
 * @param {ActiveEgress[]} props.activeEgress
 * @param {() => void} props.onOpen
 */
export function EgressBadge({ activeEgress, onOpen }) {
  if (activeEgress.length === 0) {
    return null;
  }
  return (
    <button
      type="button"
      className="rr-locked"
      style={BADGE_STYLE}
      aria-label={egressDescription(activeEgress)}
      title={egressDescription(activeEgress)}
      onClick={onOpen}
    >
      <span aria-hidden="true">{"↑"}</span>
      <span>{badgeText(activeEgress)}</span>
    </button>
  );
}

/**
 * The badge's short text: the first host, and how many more.
 * @param {ActiveEgress[]} activeEgress
 * @returns {string}
 */
export function badgeText(activeEgress) {
  const hosts = uniqueHosts(activeEgress);
  if (hosts.length === 1) {
    return `Sending data to ${hosts[0]}`;
  }
  return `Sending data to ${hosts[0]} and ${hosts.length - 1} more`;
}

/**
 * Every {purpose, host} pair in one sentence, for screen readers and the tooltip.
 * @param {ActiveEgress[]} activeEgress
 * @returns {string}
 */
export function egressDescription(activeEgress) {
  const pairs = activeEgress.map((egress) => `${egress.host} (${egress.purpose})`);
  return `Data is leaving this machine: ${pairs.join(", ")}`;
}

/**
 * @param {ActiveEgress[]} activeEgress
 * @returns {string[]}
 */
function uniqueHosts(activeEgress) {
  return [...new Set(activeEgress.map((egress) => egress.host))];
}
