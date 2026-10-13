// Basic and Advanced (layout frontend-and-plugins.md 2.4; design 3.4). The global mode is ui.mode; a panel may override
// it with its own mode_override ("" follows the global one). Pure functions.

import { DEFAULT_MODE, SECTION_LABELS } from "../constants.js";
import { readSetting } from "./settings.js";

/** @typedef {import("@rastai/ui").UiMode} UiMode */
/** @typedef {import("@rastai/ui").ModeOverride} ModeOverride */

const BASIC = "basic";
const ADVANCED = "advanced";

/**
 * @param {any} values   the settings.get result's `values`
 * @returns {UiMode}
 */
export function globalMode(values) {
  const mode = readSetting(values, "ui.mode", DEFAULT_MODE);
  return mode === BASIC ? BASIC : ADVANCED;
}

/**
 * The mode a panel shows: its own override when it has one, else the global mode.
 * @param {UiMode} globalModeValue
 * @param {"" | UiMode} override
 * @returns {UiMode}
 */
export function effectiveMode(globalModeValue, override) {
  return override === BASIC || override === ADVANCED ? override : globalModeValue;
}

/**
 * A section's name in the rail and its panel title. The label follows the effective mode: Inference's is "Chat" in
 * Basic and "Inference" in Advanced (design decision 9).
 * @param {string} sectionId
 * @param {UiMode} mode
 * @returns {string}
 */
export function sectionLabel(sectionId, mode) {
  const label = /** @type {Object<string, { basic: string, advanced: string }>} */ (SECTION_LABELS)[sectionId];
  if (label === undefined) {
    return sectionId;
  }
  return mode === BASIC ? label.basic : label.advanced;
}
