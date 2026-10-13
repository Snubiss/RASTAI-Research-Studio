// A labelled number input. Controlled; the value is a number, or null while the box is empty or not a number, so
// the caller never receives NaN.

import { useId } from "react";

import { FieldFrame, helpId } from "./FieldFrame.jsx";

/**
 * @param {object} props
 * @param {string} props.label
 * @param {number | null} props.value
 * @param {(value: number | null) => void} props.onChange
 * @param {number} [props.min]
 * @param {number} [props.max]
 * @param {number | "any"} [props.step]
 * @param {boolean} [props.required]
 * @param {string} [props.help]
 * @param {boolean} [props.disabled]
 */
export function NumberField({
  label,
  value,
  onChange,
  min,
  max,
  step = "any",
  required = false,
  help = "",
  disabled = false,
}) {
  const inputId = useId();
  return (
    <FieldFrame inputId={inputId} label={label} required={required} help={help}>
      <input
        id={inputId}
        className="rr-input"
        type="number"
        value={value === null ? "" : value}
        min={min}
        max={max}
        step={step}
        required={required}
        disabled={disabled}
        aria-describedby={help ? helpId(inputId) : undefined}
        onChange={(event) => onChange(parseNumber(event.target.value))}
      />
    </FieldFrame>
  );
}

/**
 * @param {string} text
 * @returns {number | null}
 */
export function parseNumber(text) {
  if (text.trim() === "") {
    return null;
  }
  const number = Number(text);
  return Number.isFinite(number) ? number : null;
}
