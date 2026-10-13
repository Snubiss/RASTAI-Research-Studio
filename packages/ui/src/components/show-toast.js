// The toast list as plain data: showToast adds one, dismissToast removes one. @rastai/ui keeps no global state
// (layout frontend-and-plugins.md 4.4), so the caller holds the list (apps/web keeps it in a store) and passes it
// to <Toast>.

/**
 * @typedef {object} ToastEntry
 * @property {string} id
 * @property {string} message
 * @property {"info" | "success" | "error"} tone   error borrows the locked error color
 */

// Older toasts drop off once this many are showing, so a burst of failures can't cover the window.
export const MAX_VISIBLE_TOASTS = 4;

/**
 * The list with one more toast at the end. A toast with an id already in the list replaces it (a job's progress
 * toast updates in place).
 * @param {ToastEntry[]} toasts
 * @param {ToastEntry} toast
 * @returns {ToastEntry[]}
 */
export function showToast(toasts, toast) {
  const others = toasts.filter((entry) => entry.id !== toast.id);
  const next = [...others, toast];
  return next.slice(-MAX_VISIBLE_TOASTS);
}

/**
 * @param {ToastEntry[]} toasts
 * @param {string} toastId
 * @returns {ToastEntry[]}
 */
export function dismissToast(toasts, toastId) {
  return toasts.filter((entry) => entry.id !== toastId);
}
