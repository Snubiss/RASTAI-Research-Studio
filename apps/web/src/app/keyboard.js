// The window's global keys (design 3.7). One pure function turns a key press into an action; the App listens to the
// window's keydown and runs the action. Keys typed into a text field never trigger these, except Ctrl K (the palette
// works from anywhere, as the design says), and "G then a letter" waits for its letter for GO_PREFIX_TIMEOUT_MS.

import { GO_KEYS } from "../constants.js";

/** The actions the App runs. "go:<section_id>" moves to a section. */
export const ACTION_PALETTE = "palette";
export const ACTION_SHORTCUTS = "shortcuts";
export const ACTION_FOCUS_REGION = "focus_region";
export const ACTION_GO_PREFIX = "go:";
export const ACTION_NONE = "";

/**
 * The key facts the rules read, from a KeyboardEvent.
 * @typedef {object} KeyFacts
 * @property {string} key
 * @property {boolean} ctrlOrMeta   Ctrl on Windows and Linux, Cmd on macOS
 * @property {boolean} alt
 * @property {boolean} inTextField  the focus is in an input, a textarea, a select or an editable area
 */

/**
 * @typedef {object} KeyStep
 * @property {string} action        one of the ACTION_ constants, or "go:<section_id>"
 * @property {boolean} goPending    whether the next letter goes to a section (G was pressed)
 */

/**
 * What a key press does, given whether "G" came just before it.
 * @param {KeyFacts} press
 * @param {boolean} goPending
 * @returns {KeyStep}
 */
export function keyStep(press, goPending) {
  const lowerKey = press.key.toLowerCase();
  if (press.ctrlOrMeta && lowerKey === "k") {
    return { action: ACTION_PALETTE, goPending: false };
  }
  // F6 is a function key, not a letter: it works from inside a field too, so a select or an input never traps it.
  if (press.key === "F6" && !press.ctrlOrMeta && !press.alt) {
    return { action: ACTION_FOCUS_REGION, goPending: false };
  }
  if (press.inTextField || press.ctrlOrMeta || press.alt) {
    return { action: ACTION_NONE, goPending: false };
  }
  if (goPending && Object.hasOwn(GO_KEYS, lowerKey)) {
    return { action: ACTION_GO_PREFIX + GO_KEYS[lowerKey], goPending: false };
  }
  if (press.key === "?") {
    return { action: ACTION_SHORTCUTS, goPending: false };
  }
  if (lowerKey === "g") {
    return { action: ACTION_NONE, goPending: true };
  }
  return { action: ACTION_NONE, goPending: false };
}

/**
 * The shortcut table the Shift ? list shows: the global keys of design 3.7 that this version has.
 * @type {{ keys: string, action: string }[]}
 */
export const SHORTCUT_LIST = [
  { keys: "Ctrl K", action: "Open the command palette" },
  { keys: "Esc", action: "Close the palette, a dialog or a menu" },
  { keys: "Shift ?", action: "Show this list" },
  { keys: "G then H, C, T, D, P or S", action: "Go to Home, Chat, Training, Designer, Psychology or Settings" },
  { keys: "F6", action: "Move focus between the section rail, the panels and the status bar" },
  { keys: "Ctrl .", action: "In the palette: copy the highlighted command as a CLI line" },
];

/**
 * Whether the focus is somewhere a letter is typed, so single-letter keys must leave it alone.
 * @param {EventTarget | null} target
 * @returns {boolean}
 */
export function isTextField(target) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  if (target.isContentEditable) {
    return true;
  }
  return (
    target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
  );
}
