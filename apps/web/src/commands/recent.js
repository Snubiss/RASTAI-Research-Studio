// The palette's recently run entries (layout frontend-and-plugins.md 2.5 step 5). They live in localStorage, and that is
// only a convenience: losing them changes the ranking, nothing else. Every read and write is wrapped, because a private
// window or blocked site data can make localStorage throw.

const STORAGE_KEY = "rastai.palette.recent";
const MAX_RECENT = 8;

/**
 * The ids run most recently, newest first. Anything unreadable is no history.
 * @returns {string[]}
 */
export function loadRecentIds() {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((id) => typeof id === "string").slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}

/**
 * Moves an id to the front of the history, keeping the list short.
 * @param {string} entryId
 * @param {string[]} recentIds   the history before this run
 * @returns {string[]} the history after it
 */
export function withRecentId(entryId, recentIds) {
  return [entryId, ...recentIds.filter((id) => id !== entryId)].slice(0, MAX_RECENT);
}

/**
 * Saves the history. A failed write is ignored: the palette keeps working without it.
 * @param {string[]} recentIds
 */
export function saveRecentIds(recentIds) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(recentIds));
  } catch {
    // No history this time; the ranking is only a convenience.
  }
}
