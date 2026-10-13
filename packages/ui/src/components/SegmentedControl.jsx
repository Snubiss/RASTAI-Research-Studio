// Two to four mutually exclusive options (ui-design.md 5.2): Basic or Advanced, a runtime toggle, a log filter. Each
// option is a button with aria-pressed, and the group is one Tab stop: the pressed option is the one Tab reaches, and
// the arrow keys move focus between the others (Home and End go to the ends). Space or Enter presses the focused
// option, as for any button; the arrows never press, so moving through the group changes nothing.

import { useRef } from "react";

// The keys that move to the next option, and to the previous one (both axes, since a group may be a row or a column).
const NEXT_KEYS = ["ArrowRight", "ArrowDown"];
const PREVIOUS_KEYS = ["ArrowLeft", "ArrowUp"];

/**
 * @typedef {object} SegmentOption
 * @property {string} value
 * @property {string} label
 */

/**
 * @param {object} props
 * @param {string} props.label                   the group's name for screen readers, such as "Log stream"
 * @param {SegmentOption[]} props.options
 * @param {string} props.value                   the value of the pressed option
 * @param {(value: string) => void} props.onChange
 * @param {boolean} [props.disabled]             every option is disabled: nothing presses, and the text is greyed
 */
export function SegmentedControl({ label, options, value, onChange, disabled = false }) {
  const groupRef = useRef(/** @type {HTMLDivElement | null} */ (null));
  const pressedIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  /** @param {import("react").KeyboardEvent} event */
  function handleKeyDown(event) {
    const buttons = optionButtons(groupRef.current);
    const current = buttons.indexOf(/** @type {HTMLButtonElement} */ (event.target));
    if (current < 0) {
      return;
    }
    const target = targetIndex(event.key, current, buttons.length);
    if (target !== null) {
      event.preventDefault();
      buttons[target].focus();
    }
  }

  return (
    <div ref={groupRef} className="rr-segmented" role="group" aria-label={label} onKeyDown={handleKeyDown}>
      {options.map((option, index) => (
        <button
          key={option.value}
          type="button"
          className="rr-segmented__option"
          aria-pressed={option.value === value}
          tabIndex={index === pressedIndex ? 0 : -1}
          disabled={disabled}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/**
 * The option buttons of a group, in order.
 * @param {HTMLElement | null} group
 * @returns {HTMLButtonElement[]}
 */
function optionButtons(group) {
  if (!group) {
    return [];
  }
  return [...group.querySelectorAll("button")];
}

/**
 * Which option a key moves focus to, or null when the key does nothing here. Arrows wrap at the ends.
 * @param {string} key
 * @param {number} current
 * @param {number} count
 * @returns {number | null}
 */
function targetIndex(key, current, count) {
  if (NEXT_KEYS.includes(key)) {
    return (current + 1) % count;
  }
  if (PREVIOUS_KEYS.includes(key)) {
    return (current - 1 + count) % count;
  }
  if (key === "Home") {
    return 0;
  }
  if (key === "End") {
    return count - 1;
  }
  return null;
}
