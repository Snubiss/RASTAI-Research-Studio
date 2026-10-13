// A labelled text input, or a text area with `multiline`. Controlled: the caller owns the value.

import { useId } from "react";

import { FieldFrame, helpId } from "./FieldFrame.jsx";

/**
 * @param {object} props
 * @param {string} props.label
 * @param {string} props.value
 * @param {(value: string) => void} props.onChange
 * @param {boolean} [props.required]
 * @param {string} [props.help]
 * @param {string} [props.placeholder]
 * @param {boolean} [props.multiline]
 * @param {boolean} [props.monospace]   for JSON and code
 * @param {boolean} [props.disabled]
 */
export function TextField({
  label,
  value,
  onChange,
  required = false,
  help = "",
  placeholder = "",
  multiline = false,
  monospace = false,
  disabled = false,
}) {
  const inputId = useId();
  const className = monospace ? "rr-input rr-input--mono" : "rr-input";
  const shared = {
    id: inputId,
    className,
    value,
    required,
    placeholder,
    disabled,
    "aria-describedby": help ? helpId(inputId) : undefined,
  };
  return (
    <FieldFrame inputId={inputId} label={label} required={required} help={help}>
      {multiline ? (
        <textarea {...shared} rows={4} onChange={(event) => onChange(event.target.value)} />
      ) : (
        <input {...shared} type="text" onChange={(event) => onChange(event.target.value)} />
      )}
    </FieldFrame>
  );
}
