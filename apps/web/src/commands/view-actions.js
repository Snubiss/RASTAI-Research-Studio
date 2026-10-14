// View actions (layout frontend-and-plugins.md 2.5 step 3): palette entries that only move the window, to a section.
// They never reach the daemon and have no "Copy as". The section labels follow the mode, as the rail's do.

import { sectionLabel } from "../state/mode.js";

export const VIEW_SECTION_OPEN_PREFIX = "view.section.open:";

/**
 * One "Open <section>" entry per section.
 * @param {string[]} sectionIds
 * @param {"basic" | "advanced"} mode
 * @returns {{ id: string, sectionId: string, title: string }[]}
 */
export function sectionViewActions(sectionIds, mode) {
  return sectionIds.map((sectionId) => ({
    id: `${VIEW_SECTION_OPEN_PREFIX}${sectionId}`,
    sectionId,
    title: `Open ${sectionLabel(sectionId, mode)}`,
  }));
}
