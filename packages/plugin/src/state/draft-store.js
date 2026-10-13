// Unsent drafts (layout frontend-and-plugins.md 2.3; spec §2: nothing the user did is lost to a crash): text the
// user typed but hasn't sent, kept in localStorage so a WebKitGTK crash, a reload or a resync doesn't lose a long
// prompt. A safety net, not storage: every access is wrapped in try/catch, a sandboxed iframe (where localStorage
// throws) simply keeps no drafts, and nothing else may go here (never settings, secrets or incognito text).
// Keys are `draft:<section>:<subject_id>`, e.g. `draft:inference:new`.

const DRAFT_SAVE_DELAY_MS = 500; // one write per typing pause, not per keystroke
const DRAFT_MAX_AGE_DAYS = 30; // older drafts are dropped on load
const MILLISECONDS_PER_DAY = 24 * 60 * 60 * 1000;

/**
 * What one stored draft holds.
 * @typedef {object} StoredDraft
 * @property {string} text
 * @property {number} savedUnixMs
 */

/** @type {Map<string, ReturnType<typeof setTimeout>>} */
const pendingSaves = new Map();

/**
 * @param {string} key
 * @returns {string | null} null when missing, too old, or storage is unavailable
 */
export function loadDraft(key) {
  const draft = readDraft(key);
  if (draft === null) {
    return null;
  }
  if (Date.now() - draft.savedUnixMs > DRAFT_MAX_AGE_DAYS * MILLISECONDS_PER_DAY) {
    removeStored(key);
    return null;
  }
  return draft.text;
}

/**
 * Debounced by DRAFT_SAVE_DELAY_MS; empty text clears the draft.
 * @param {string} key
 * @param {string} text
 */
export function saveDraft(key, text) {
  cancelPendingSave(key);
  const timer = setTimeout(() => {
    pendingSaves.delete(key);
    if (text === "") {
      removeStored(key);
    } else {
      writeDraft(key, { text, savedUnixMs: Date.now() });
    }
  }, DRAFT_SAVE_DELAY_MS);
  pendingSaves.set(key, timer);
}

/**
 * Called once the send or edit that commits the text returns ok. A save still waiting is dropped too, so it can't
 * bring the draft back afterwards.
 * @param {string} key
 */
export function clearDraft(key) {
  cancelPendingSave(key);
  removeStored(key);
}

/** @param {string} key */
function cancelPendingSave(key) {
  const timer = pendingSaves.get(key);
  if (timer !== undefined) {
    clearTimeout(timer);
    pendingSaves.delete(key);
  }
}

/**
 * @param {string} key
 * @returns {StoredDraft | null}
 */
function readDraft(key) {
  try {
    const stored = localStorage.getItem(key);
    if (stored === null) {
      return null;
    }
    const draft = JSON.parse(stored);
    const isDraft = typeof draft?.text === "string" && typeof draft?.savedUnixMs === "number";
    return isDraft ? draft : null;
  } catch {
    // No storage here (a sandboxed iframe, storage turned off) or a value that isn't ours: no draft.
    return null;
  }
}

/**
 * @param {string} key
 * @param {StoredDraft} draft
 */
function writeDraft(key, draft) {
  try {
    localStorage.setItem(key, JSON.stringify(draft));
  } catch {
    // Storage full or unavailable: the draft is a convenience, so the app goes on without it.
  }
}

/** @param {string} key */
function removeStored(key) {
  try {
    localStorage.removeItem(key);
  } catch {
    // Unavailable storage holds nothing to remove.
  }
}
