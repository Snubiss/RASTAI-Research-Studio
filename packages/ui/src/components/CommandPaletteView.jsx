// The command palette's view only: the filter box, the list and keyboard navigation (layout
// frontend-and-plugins.md 2.5). Building, filtering and ranking the entries is apps/web's job; this component
// shows what it is given. Up and Down move the highlight, Enter runs the highlighted entry, Escape closes.

import { useEffect, useId, useState } from "react";

/**
 * One palette row, already filtered and ranked by the caller.
 * @typedef {object} PaletteEntry
 * @property {string} id         a command id or a view action id
 * @property {string} title
 * @property {string} [detail]   the section or a short description
 * @property {string} [shortcut] as the user presses it, e.g. "Ctrl+K"
 * @property {string} [note]     a short state, such as "Needs input"
 */

/**
 * @param {object} props
 * @param {PaletteEntry[]} props.entries
 * @param {string} props.query
 * @param {(query: string) => void} props.onQueryChange
 * @param {(entryId: string) => void} props.onRun
 * @param {() => void} props.onClose
 * @param {(entryId: string) => void} [props.onHighlight]   the highlighted entry's id, or "" for none; the caller's
 *                                                         detail pane follows it
 * @param {string} [props.label]         the filter box's accessible name
 * @param {string} [props.emptyText]
 */
export function CommandPaletteView({
  entries,
  query,
  onQueryChange,
  onRun,
  onClose,
  onHighlight,
  label = "Run a command",
  emptyText = "No matching commands.",
}) {
  // The highlight is kept with the list it was set on. A new list has no highlight, so its first entry is the highlighted one
  // in the same render: a key pressed before any effect has run (typing, then Enter) acts on the list on screen.
  const [highlight, setHighlight] = useState({ list: entries, index: 0 });
  const activeIndex = highlight.list === entries ? highlight.index : 0;
  /** @param {number} index */
  function moveHighlight(index) {
    setHighlight({ list: entries, index });
  }
  const listId = useId();

  // The caller shows the highlighted entry's details, so it hears about every move of the highlight.
  useEffect(() => {
    onHighlight?.(entries[activeIndex]?.id ?? "");
  }, [entries, activeIndex]);

  /** @param {import("react").KeyboardEvent} event */
  function handleKeyDown(event) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveHighlight(nextIndex(activeIndex, 1, entries.length));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveHighlight(nextIndex(activeIndex, -1, entries.length));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (entries[activeIndex]) {
        onRun(entries[activeIndex].id);
      }
    } else if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  }

  const activeEntry = entries[activeIndex];
  return (
    <div className="rr-palette">
      <input
        className="rr-input rr-palette__input"
        type="text"
        role="combobox"
        aria-label={label}
        aria-expanded="true"
        aria-controls={listId}
        aria-activedescendant={activeEntry ? optionId(listId, activeIndex) : undefined}
        value={query}
        autoFocus
        onChange={(event) => onQueryChange(event.target.value)}
        onKeyDown={handleKeyDown}
      />
      <ul className="rr-palette__list" id={listId} role="listbox" aria-label={label}>
        {entries.map((entry, index) => (
          <li
            key={entry.id}
            id={optionId(listId, index)}
            role="option"
            aria-selected={index === activeIndex}
            className={index === activeIndex ? "rr-palette__entry rr-palette__entry--active" : "rr-palette__entry"}
            onMouseMove={() => moveHighlight(index)}
            onClick={() => onRun(entry.id)}
          >
            <span className="rr-palette__title">{entry.title}</span>
            {entry.detail ? <span className="rr-palette__detail">{entry.detail}</span> : null}
            {entry.note ? <span className="rr-palette__detail">{entry.note}</span> : null}
            {entry.shortcut ? <kbd className="rr-palette__shortcut">{entry.shortcut}</kbd> : null}
          </li>
        ))}
      </ul>
      {entries.length === 0 ? <p className="rr-palette__empty">{emptyText}</p> : null}
    </div>
  );
}

/**
 * The highlight moved by one step, wrapping at both ends.
 * @param {number} index
 * @param {number} step       1 or -1
 * @param {number} count
 * @returns {number}
 */
export function nextIndex(index, step, count) {
  if (count === 0) {
    return 0;
  }
  return (index + step + count) % count;
}

/**
 * @param {string} listId
 * @param {number} index
 * @returns {string}
 */
function optionId(listId, index) {
  return `${listId}-option-${index}`;
}
