// A labelled checkbox for one yes/no value. Controlled. The label sits after the box and is part of the click
// target, so the target is wider than the box itself.

import { useId } from "react";

import { helpId } from "./FieldFrame.jsx";

/**
 * @param {object} props
 * @param {string} props.label
 * @param {boolean} props.checked
 * @param {(checked: boolean) => void} props.onChange
 * @param {string} [props.help]
 * @param {boolean} [props.disabled]
 */
export function Checkbox({ label, checked, onChange, help = "", disabled = false }) {
  const inputId = useId();
  return (
    <div className="rr-field rr-field--inline">
      <input
        id={inputId}
        className="rr-checkbox"
        type="checkbox"
        checked={checked}
        disabled={disabled}
        aria-describedby={help ? helpId(inputId) : undefined}
        onChange={(event) => onChange(event.target.checked)}
      />
      <label className="rr-field__label" htmlFor={inputId}>
        {label}
      </label>
      {help ? (
        <p className="rr-field__help" id={helpId(inputId)}>
          {help}
        </p>
      ) : null}
    </div>
  );
}
