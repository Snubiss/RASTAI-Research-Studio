// The command palette (layout frontend-and-plugins.md 2.5; design 3.6, 6.3). Ctrl K or the top bar opens it. Two panes:
// the result list, with the filter, the context chips and the footer, and the detail pane for the highlighted entry.
// The entries are built from the daemon's ListCommands descriptors, the window's context and the mode; this file only
// shows them and runs what the person picks. Escape closes it and returns the focus.

import { CommandPaletteView, Button, Dialog } from "@rastai/ui";
import { useStore } from "@rastai/plugin";
import { useMemo, useRef, useState } from "react";

import { useShell } from "../app/shell-context.js";
import { HOME_SECTION_ID, SECTION_GROUPS } from "../constants.js";
import { copyAsFormats } from "../commands/copy-as.js";
import { contextChips, flatContext } from "../commands/context-arguments.js";
import { COPY_AS_CLI_SHORTCUT, shortcutMatches } from "../commands/keybindings.js";
import { buildPaletteItems } from "../commands/palette-entries.js";
import { loadRecentIds, saveRecentIds, withRecentId } from "../commands/recent.js";
import { paletteHostOf, runFromPalette } from "../commands/run-command.js";
import { navigateTo } from "../routing/navigate.js";
import { sectionRoute } from "../routing/route.js";
import { readSetting } from "../state/settings.js";
import { PaletteDetail } from "./PaletteDetail.jsx";

/** @typedef {import("../commands/palette-entries.js").PaletteItem} PaletteItem */

// Every section the palette can open, Home first, in the rail's order.
const SECTION_IDS = [HOME_SECTION_ID, ...SECTION_GROUPS.flatMap((group) => group.sectionIds)];
const EXPERIMENTAL_SETTING = "plugins.experimental_apis";
const COPY_CLI_DONE = "Copied the CLI line.";
const COPY_DONE = "Copied.";
const COPY_FAILED = "Copy did not work. Select the text and copy it by hand.";
const KEY_HINTS = "↑ ↓ move · Enter run · Tab into arguments · Ctrl . copy as CLI · Esc close";
const NO_COMMANDS_TEXT = "The command list isn't loaded, so only the views are listed.";

export function CommandPalette() {
  const shell = useShell();
  const palette = useStore(shell.stores.palette);
  const display = useStore(shell.stores.display);
  const commands = useStore(shell.stores.commands);
  const settings = useStore(shell.stores.settings);
  const context = useStore(shell.stores.context);
  const route = useStore(shell.stores.route);
  const [highlightedId, setHighlightedId] = useState("");
  const [showAll, setShowAll] = useState(false);
  const [recentIds, setRecentIds] = useState(() => loadRecentIds());
  // The arguments each command's form holds, by command id, so Copy as writes what the form shows.
  const [argumentsById, setArgumentsById] = useState(/** @type {Object<string, Object<string, any>>} */ ({}));
  const [copyFormat, setCopyFormat] = useState("cli");
  const [copyStatus, setCopyStatus] = useState("");
  const detailRef = useRef(/** @type {HTMLDivElement | null} */ (null));
  const paletteHost = useMemo(() => paletteHostOf(shell), [shell]);

  const experimentalOn = readSetting(settings.values, EXPERIMENTAL_SETTING, false) === true;
  const { items, hiddenCount } = useMemo(
    () =>
      buildPaletteItems({
        commands,
        sectionIds: SECTION_IDS,
        mode: display.mode,
        query: palette.query,
        recentIds,
        currentSectionId: route.sectionId,
        showAll,
        experimentalOn,
        context: flatContext(context),
      }),
    [commands, display.mode, palette.query, recentIds, route.sectionId, showAll, experimentalOn, context],
  );
  const selected = items.find((item) => item.id === highlightedId) ?? items[0] ?? null;

  function close() {
    shell.stores.palette.set({ open: false, query: "" });
    setShowAll(false);
  }

  /** @param {string} query */
  function changeQuery(query) {
    shell.stores.palette.set({ open: true, query });
  }

  /** @param {string} entryId */
  function runEntry(entryId) {
    const item = items.find((candidate) => candidate.id === entryId);
    if (!item) {
      return;
    }
    if (item.isView) {
      close();
      navigateTo(sectionRoute(item.sectionId));
    } else if (item.note !== "") {
      // Needs input: the form is in the detail pane, so Enter moves the focus into it.
      /** @type {HTMLElement | null | undefined} */ (
        detailRef.current?.querySelector("input, select, textarea, button")
      )?.focus();
    } else {
      runCommandItem(item);
    }
  }

  /** @param {PaletteItem} item */
  async function runCommandItem(item) {
    remember(item.id);
    close();
    if (item.command) {
      await runFromPalette(shell, item.command, item.prefilled);
    }
  }

  /** @param {string} itemId */
  function formRan(itemId) {
    remember(itemId);
    close();
  }

  /** @param {string} itemId */
  function remember(itemId) {
    const next = withRecentId(itemId, recentIds);
    setRecentIds(next);
    saveRecentIds(next);
  }

  /** @param {string} itemId @param {Object<string, any>} args */
  function setArgumentsFor(itemId, args) {
    setArgumentsById((current) => ({ ...current, [itemId]: args }));
  }

  /**
   * Copies the highlighted command's call in one format. The shortcut copies the CLI line; the detail's button copies
   * the format it shows.
   * @param {string} format   a COPY_FORMATS value
   * @param {string} doneText
   */
  async function copySelected(format, doneText) {
    if (selected && !selected.isView) {
      await copyText(copyAsFormats(selected.id, argumentsOf(selected))[format], doneText);
    }
  }

  /** @param {PaletteItem} item */
  function argumentsOf(item) {
    return argumentsById[item.id] ?? item.prefilled;
  }

  /** @param {string} text @param {string} doneText */
  async function copyText(text, doneText) {
    try {
      await navigator.clipboard.writeText(text);
      setCopyStatus(doneText);
    } catch {
      setCopyStatus(COPY_FAILED);
    }
  }

  /** @param {import("react").KeyboardEvent} event */
  function handleKeyDown(event) {
    if (shortcutMatches(COPY_AS_CLI_SHORTCUT, event)) {
      event.preventDefault();
      copySelected("cli", COPY_CLI_DONE);
    }
  }

  return (
    <Dialog open={palette.open} title="Command palette" onClose={close}>
      <div className="rr-palette-layout" onKeyDown={handleKeyDown}>
        <div className="rr-palette-layout__list">
          <CommandPaletteView
            entries={items}
            query={palette.query}
            onQueryChange={changeQuery}
            onRun={runEntry}
            onClose={close}
            onHighlight={setHighlightedId}
            label="Search or run a command"
            emptyText={emptyText(palette.query, hiddenCount, showAll)}
          />
          {commands.length === 0 ? <p className="rr-palette__empty">{NO_COMMANDS_TEXT}</p> : null}
          <PaletteFooter
            hiddenCount={hiddenCount}
            showAll={showAll}
            onShowAll={() => setShowAll(true)}
            shownCount={items.length}
          />
          {selected && !selected.isView ? (
            <div className="rr-palette__chips" role="group" aria-label="Filled from the context">
              {contextChips(selected.prefilled).map((chip) => (
                <span key={chip.name} className="rr-palette__chip">
                  {`${chip.name} = ${chip.value}`}
                </span>
              ))}
            </div>
          ) : null}
        </div>
        <div className="rr-palette-layout__detail" ref={detailRef}>
          <PaletteDetail
            item={selected}
            host={paletteHost}
            copyFormat={copyFormat}
            onCopyFormatChange={setCopyFormat}
            copyStatus={copyStatus}
            onCopy={() => copySelected(copyFormat, COPY_DONE)}
            argumentsOfItem={selected ? argumentsOf(selected) : {}}
            onArgumentsChange={setArgumentsFor}
            onRun={runEntry}
            onFormRan={formRan}
          />
        </div>
      </div>
    </Dialog>
  );
}

/**
 * The footer: the keys, how many rows are shown, and the way to show the advanced commands Basic hides.
 * @param {object} props
 * @param {number} props.hiddenCount
 * @param {boolean} props.showAll
 * @param {() => void} props.onShowAll
 * @param {number} props.shownCount
 */
function PaletteFooter({ hiddenCount, showAll, onShowAll, shownCount }) {
  return (
    <footer className="rr-palette__footer">
      <p className="rr-palette__hints">{KEY_HINTS}</p>
      <p className="rr-palette__count">{`${shownCount} shown`}</p>
      {hiddenCount > 0 && !showAll ? (
        <Button variant="quiet" onClick={onShowAll}>
          {`${hiddenCount} more in Advanced`}
        </Button>
      ) : null}
    </footer>
  );
}

/**
 * What the list says when nothing matches. A Basic query that hides matches says how to search them.
 * @param {string} query
 * @param {number} hiddenCount
 * @param {boolean} showAll
 * @returns {string}
 */
function emptyText(query, hiddenCount, showAll) {
  if (query.trim() === "") {
    return "No commands to show.";
  }
  const hint = hiddenCount > 0 && !showAll ? " Search all modes to include the advanced commands." : "";
  return `No command matches "${query}".${hint}`;
}
