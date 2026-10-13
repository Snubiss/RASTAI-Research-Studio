// The label, required mark and help text around one form control, so every field lays out the same way. Private
// to components/: TextField, NumberField, Select and Checkbox use it.

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {string} props.inputId      the control's id, which the label points at
 * @param {string} props.label
 * @param {boolean} [props.required]
 * @param {string} [props.help]
 * @param {ReactNode} props.children   the control
 */
export function FieldFrame({ inputId, label, required = false, help = "", children }) {
  return (
    <div className="rr-field">
      <label className="rr-field__label" htmlFor={inputId}>
        {label}
        {required ? (
          <span className="rr-field__required" aria-hidden="true">
            {" *"}
          </span>
        ) : null}
      </label>
      {children}
      {help ? (
        <p className="rr-field__help" id={helpId(inputId)}>
          {help}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The id of a field's help text, for the control's aria-describedby.
 * @param {string} inputId
 * @returns {string}
 */
export function helpId(inputId) {
  return `${inputId}-help`;
}
