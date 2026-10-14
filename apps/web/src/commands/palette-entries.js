// The palette's entries (layout frontend-and-plugins.md 2.5 steps 4 to 6; design 6.3): the command descriptors and the
// view actions, filtered by mode, experimental hiding and the query, and ranked. The daemon's descriptors are the only
// commands; nothing here decides what a command does. Pure, so it is tested in Node.

import { sectionLabel } from "../state/mode.js";
import { argumentNames, requiredArgumentNames } from "./command-index.js";
import { prefillArguments } from "./context-arguments.js";
import { isMatch, matchRank } from "./palette-match.js";
import { shortcutLabel } from "./keybindings.js";
import { sectionViewActions } from "./view-actions.js";

// The order of the groups an empty query shows (design 6.3): Recent, then the current section, the rest, and the views.
const GROUP_RECENT = 0;
const GROUP_CURRENT_SECTION = 1;
const GROUP_OTHER = 2;
const GROUP_VIEW = 3;
const NOTE_NEEDS_INPUT = "Needs input";
const VIEW_DETAIL = "View";

/**
 * One row of the palette: a command, or a view action.
 * @typedef {object} PaletteItem
 * @property {string} id              the command id, or the view action's id
 * @property {string} title
 * @property {string} detail          the section's name, or "View" for a view action
 * @property {string} shortcut        as the user presses it, e.g. "Ctrl+K"; "" for none
 * @property {string} note            "Needs input" when a required argument has no value yet; "" otherwise
 * @property {boolean} isView
 * @property {string} sectionId
 * @property {any} command            the CommandDescriptor; null for a view action
 * @property {Object<string, any>} prefilled   the arguments the context fills
 */

/**
 * @typedef {object} PaletteOptions
 * @property {any[]} commands             CommandDescriptor[] (ListCommands)
 * @property {string[]} sectionIds        the sections the rail shows, Home included
 * @property {"basic" | "advanced"} mode  the effective mode of the palette
 * @property {string} query
 * @property {string[]} recentIds         the ids run most recently, newest first
 * @property {string} currentSectionId
 * @property {boolean} showAll            "Search all modes": Basic lists the commands it hides too
 * @property {boolean} experimentalOn     plugins.experimental_apis
 * @property {Object<string, any>} context   the flat context (context-arguments.js)
 */

/**
 * The rows for a query, in their order, and how many matching commands Basic mode hides.
 * @param {PaletteOptions} options
 * @returns {{ items: PaletteItem[], hiddenCount: number }}
 */
export function buildPaletteItems(options) {
  /** @type {{ rank: number, group: number, position: number, item: PaletteItem }[]} */
  const ranked = [];
  let hiddenCount = 0;
  options.commands.forEach((command, position) => {
    if (command.experimental && !options.experimentalOn) {
      return;
    }
    const rank = commandRank(command, options, sectionLabel(command.section, options.mode));
    if (rank < 0) {
      return;
    }
    if (options.mode === "basic" && !options.showAll && !command.basic) {
      hiddenCount += 1;
      return;
    }
    const item = commandItem(command, options);
    ranked.push({ rank, group: commandGroup(command, options), position, item });
  });
  for (const view of sectionViewActions(options.sectionIds, options.mode)) {
    const rank = matchRank(options.query, view.title);
    if (isMatch(rank)) {
      ranked.push({ rank, group: GROUP_VIEW, position: 0, item: viewItem(view) });
    }
  }
  return { items: sortRanked(ranked, options).map((entry) => entry.item), hiddenCount };
}

/**
 * The best rank of a command's title, id and section; -1 when none of them matches the query.
 * @param {any} command
 * @param {PaletteOptions} options
 * @param {string} sectionName
 * @returns {number}
 */
function commandRank(command, options, sectionName) {
  const ranks = [command.title, command.commandId, sectionName]
    .map((text) => matchRank(options.query, String(text ?? "")))
    .filter(isMatch);
  return ranks.length === 0 ? -1 : Math.min(...ranks);
}

/**
 * Recent first, then the current section, then the rest. A query orders by how well each entry matches, then by group.
 * @param {{ rank: number, group: number, position: number, item: PaletteItem }[]} ranked
 * @param {PaletteOptions} options
 * @returns {{ rank: number, group: number, position: number, item: PaletteItem }[]}
 */
function sortRanked(ranked, options) {
  const queried = options.query.trim() !== "";
  return [...ranked].sort((first, second) => {
    const byRank = queried ? first.rank - second.rank : 0;
    if (byRank !== 0) {
      return byRank;
    }
    const byGroup = first.group - second.group;
    if (byGroup !== 0) {
      return byGroup;
    }
    const byRecent =
      recentPosition(first.item.id, options.recentIds) - recentPosition(second.item.id, options.recentIds);
    return byRecent !== 0 ? byRecent : first.position - second.position;
  });
}

/**
 * Where an id sits in the history: its place, or a large number when it was never run, so unrun entries sort last.
 * @param {string} entryId
 * @param {string[]} recentIds
 * @returns {number}
 */
function recentPosition(entryId, recentIds) {
  const place = recentIds.indexOf(entryId);
  return place === -1 ? Number.MAX_SAFE_INTEGER : place;
}

/**
 * @param {any} command
 * @param {PaletteOptions} options
 * @returns {number}
 */
function commandGroup(command, options) {
  if (options.recentIds.includes(command.commandId)) {
    return GROUP_RECENT;
  }
  return command.section === options.currentSectionId ? GROUP_CURRENT_SECTION : GROUP_OTHER;
}

/**
 * @param {any} command
 * @param {PaletteOptions} options
 * @returns {PaletteItem}
 */
function commandItem(command, options) {
  const prefilled = prefillArguments(argumentNames(command.argumentsSchemaJson), options.context);
  const missing = requiredArgumentNames(command.argumentsSchemaJson).filter((name) => !(name in prefilled));
  return {
    id: command.commandId,
    title: command.title,
    detail: sectionLabel(command.section, options.mode),
    shortcut: shortcutLabel(command.defaultShortcut ?? ""),
    note: missing.length > 0 ? NOTE_NEEDS_INPUT : "",
    isView: false,
    sectionId: command.section,
    command,
    prefilled,
  };
}

/**
 * @param {{ id: string, sectionId: string, title: string }} view
 * @returns {PaletteItem}
 */
function viewItem(view) {
  return {
    id: view.id,
    title: view.title,
    detail: VIEW_DETAIL,
    shortcut: "",
    note: "",
    isView: true,
    sectionId: view.sectionId,
    command: null,
    prefilled: {},
  };
}
