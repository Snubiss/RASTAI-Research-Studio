// The command palette's entry point (layout frontend-and-plugins.md 2.5; design 3.6). This version lists the sections
// and nothing else; M1-54 adds the commands, the copy-as block and the Basic filter. It opens from Ctrl K and the top
// bar's entry, and Escape closes it through the dialog.

import { CommandPaletteView, Dialog } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { useShell } from "../app/shell-context.js";
import { SECTION_GROUPS, HOME_SECTION_ID } from "../constants.js";
import { navigateTo } from "../routing/navigate.js";
import { sectionRoute } from "../routing/route.js";
import { sectionLabel } from "../state/mode.js";
import { matchesQuery } from "./palette-match.js";

const SECTION_ENTRY_PREFIX = "section:";

export function CommandPalette() {
  const shell = useShell();
  const palette = useStore(shell.stores.palette);
  const display = useStore(shell.stores.display);
  const entries = sectionEntries(display.mode, palette.query);
  const close = () => shell.stores.palette.set({ open: false, query: "" });

  /** @param {string} entryId */
  function run(entryId) {
    const sectionId = entryId.slice(SECTION_ENTRY_PREFIX.length);
    close();
    navigateTo(sectionRoute(sectionId));
  }

  return (
    <Dialog open={palette.open} title="Command palette" onClose={close}>
      <CommandPaletteView
        entries={entries}
        query={palette.query}
        onQueryChange={(query) => shell.stores.palette.set({ open: true, query })}
        onRun={run}
        onClose={close}
      />
    </Dialog>
  );
}

/**
 * The section entries whose label matches the query, Home first.
 * @param {"basic" | "advanced"} mode
 * @param {string} query
 * @returns {{ id: string, title: string, detail: string }[]}
 */
export function sectionEntries(mode, query) {
  const sectionIds = [HOME_SECTION_ID, ...SECTION_GROUPS.flatMap((group) => group.sectionIds)];
  return sectionIds
    .map((sectionId) => ({
      id: `${SECTION_ENTRY_PREFIX}${sectionId}`,
      title: sectionLabel(sectionId, mode),
      detail: sectionId,
    }))
    .filter((entry) => matchesQuery(query, entry.title));
}
