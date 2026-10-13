// A labelled drop-down list of choices. Controlled. A native <select>, so keyboard and screen readers work the way
// each platform expects.

import { useId } from "react";

import { FieldFrame, helpId } from "./FieldFrame.jsx";

/**
 * One choice: what is stored and what is shown.
 * @typedef {object} SelectOption
 * @property {string} value
 * @property {string} label
 */

/**
 * @param {object} props
 * @param {string} props.label
 * @param {string} props.value
 * @param {SelectOption[]} props.options
 * @param {(value: string) => void} props.onChange
 * @param {boolean} [props.required]
 * @param {string} [props.help]
 * @param {boolean} [props.disabled]
 */
export function Select({ label, value, options, onChange, required = false, help = "", disabled = false }) {
  const inputId = useId();
  return (
    <FieldFrame inputId={inputId} label={label} required={required} help={help}>
      <select
        id={inputId}
        className="rr-input rr-select"
        value={value}
        required={required}
        disabled={disabled}
        aria-describedby={help ? helpId(inputId) : undefined}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </FieldFrame>
  );
}
