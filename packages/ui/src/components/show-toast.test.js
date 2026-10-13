// The toast list as plain data (show-toast.js).

import { expect, test } from "vitest";

import { MAX_VISIBLE_TOASTS, dismissToast, showToast } from "./show-toast.js";

/**
 * @param {string} id
 * @returns {import("./show-toast.js").ToastEntry}
 */
function toast(id) {
  return { id, message: `toast ${id}`, tone: "info" };
}

test("adds at the end, replaces a toast with the same id, and keeps only the newest few", () => {
  let toasts = showToast([], toast("a"));
  toasts = showToast(toasts, toast("b"));
  toasts = showToast(toasts, { ...toast("a"), message: "updated" });
  expect(toasts.map((entry) => entry.id)).toEqual(["b", "a"]);
  expect(toasts[1].message).toBe("updated");
  for (let index = 0; index < MAX_VISIBLE_TOASTS + 2; index += 1) {
    toasts = showToast(toasts, toast(`n${index}`));
  }
  expect(toasts).toHaveLength(MAX_VISIBLE_TOASTS);
  expect(toasts.at(-1)?.id).toBe(`n${MAX_VISIBLE_TOASTS + 1}`);
});

test("dismisses by id and leaves the list it was given alone", () => {
  const toasts = [toast("a"), toast("b")];
  expect(dismissToast(toasts, "a").map((entry) => entry.id)).toEqual(["b"]);
  expect(toasts).toHaveLength(2);
});
