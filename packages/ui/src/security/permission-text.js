// Plugin permissions in plain English, for the trust prompt and the permission list (contracts 6.3; spec §24).
// gpu, weights.read and conversations.read are the sensitive three. A permission this table doesn't know is shown
// as it is and marked sensitive, so a new or misspelled permission is never played down.

/**
 * @typedef {object} PermissionText
 * @property {string} permission   as the manifest writes it, e.g. "network:[huggingface.co]"
 * @property {string} text
 * @property {boolean} sensitive
 */

const LIST_PERMISSIONS = {
  commands: "Run commands that need these permissions: ",
  network: "Connect to these internet hosts from its Python code: ",
};
/** @type {Object<string, { text: string, sensitive: boolean }>} */
const PLAIN_PERMISSIONS = {
  "fs:project|ro": { text: "Read the files of the open project (read-only)", sensitive: false },
  secrets: { text: "Use your stored secrets and sign-ins", sensitive: false },
  gpu: { text: "Use your GPU", sensitive: true },
  "weights.read": { text: "Read the weights of loaded models", sensitive: true },
  "conversations.read": { text: "Read your conversation history", sensitive: true },
  "gateway.spend": { text: "Call paid third-party APIs", sensitive: false },
  "sandbox.spawn": { text: "Create and remove sandboxes", sensitive: false },
};
// "commands:[read, chat]" -> "commands" and "read, chat".
const LIST_PERMISSION = /^([a-z]+):\[(.*)\]$/;

/**
 * @param {string} permission
 * @returns {PermissionText}
 */
export function permissionText(permission) {
  const plain = PLAIN_PERMISSIONS[permission];
  if (plain) {
    return { permission, text: plain.text, sensitive: plain.sensitive };
  }
  const listMatch = LIST_PERMISSION.exec(permission);
  if (listMatch && (listMatch[1] === "commands" || listMatch[1] === "network")) {
    const items = listMatch[2]
      .split(",")
      .map((item) => item.trim())
      .filter((item) => item !== "");
    return { permission, text: `${LIST_PERMISSIONS[listMatch[1]]}${items.join(", ")}`, sensitive: false };
  }
  return { permission, text: `Unknown permission: ${permission}`, sensitive: true };
}
