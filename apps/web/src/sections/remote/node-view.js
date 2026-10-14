// What the Remote list says about one node (ui-design 6.8). node.list says what the nodes are; these functions turn it
// into words, a tone and the heartbeat's age. The local node is the one this computer runs.

const LOCAL_TRANSPORT = "local";
const CONNECTED_STATE = "connected";
const UNREACHABLE_STATE = "unreachable";
const MILLISECONDS_PER_SECOND = 1000;

/**
 * Where a node is, in words.
 * @param {any} node   a NodeSummary
 * @returns {string}
 */
export function nodeWhere(node) {
  return node.transport === LOCAL_TRANSPORT ? "This computer" : `SSH to ${node.address ?? "a remote computer"}`;
}

/**
 * The state pill's tone: connected is ok, unreachable an error, anything else neutral.
 * @param {string} state
 * @returns {"ok" | "error" | "neutral"}
 */
export function nodeTone(state) {
  if (state === CONNECTED_STATE) {
    return "ok";
  }
  return state === UNREACHABLE_STATE ? "error" : "neutral";
}

/**
 * How long ago the last heartbeat came, in whole seconds, or no word when none has come yet.
 * @param {number | undefined} lastHeartbeatUnixMs   0 or absent before the first heartbeat
 * @param {number} nowUnixMs
 * @returns {string}
 */
export function heartbeatWord(lastHeartbeatUnixMs, nowUnixMs) {
  if (!lastHeartbeatUnixMs) {
    return "No heartbeat yet";
  }
  const seconds = Math.max(0, Math.round((nowUnixMs - lastHeartbeatUnixMs) / MILLISECONDS_PER_SECOND));
  return `Last heartbeat ${seconds} s ago`;
}
