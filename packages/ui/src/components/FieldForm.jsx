// Renders a list of FieldSpec (plain data, a view model rather than a schema type) as a form (layout
// frontend-and-plugins.md 4.2). @rastai/plugin's CommandForm builds the specs from a command's arguments schema
// and uses this form. Controlled: every change calls onChange with all the values.

import { Checkbox } from "./Checkbox.jsx";
import { NumberField } from "./NumberField.jsx";
import { Select } from "./Select.jsx";
import { TextField } from "./TextField.jsx";

// A choice field with no value shows this empty option instead of the first choice, so the form never shows a value
// it doesn't report, and a required choice stays invalid until the user picks one.
const NO_CHOICE_LABEL = "Choose…";

/**
 * @typedef {object} FieldSpec
 * @property {string} name
 * @property {string} label
 * @property {"text" | "number" | "boolean" | "choice" | "json"} kind
 * @property {boolean} required
 * @property {string} help
 * @property {string[]} choices   for kind "choice"; empty otherwise
 */

/**
 * @param {object} props
 * @param {FieldSpec[]} props.fields
 * @param {Object<string, any>} props.values
 * @param {(values: Object<string, any>) => void} props.onChange
 */
export function FieldForm({ fields, values, onChange }) {
  /**
   * @param {string} name
   * @param {any} value
   */
  function setValue(name, value) {
    onChange({ ...values, [name]: value });
  }

  return (
    <div className="rr-form">
      {fields.map((field) => (
        <FieldControl key={field.name} field={field} value={values[field.name]} onChange={setValue} />
      ))}
    </div>
  );
}

/**
 * The one control a FieldSpec's kind calls for. A missing value shows as empty, unchecked or "Choose…". Callers
 * that want a default put it in `values`.
 * @param {object} props
 * @param {FieldSpec} props.field
 * @param {any} props.value
 * @param {(name: string, value: any) => void} props.onChange
 */
function FieldControl({ field, value, onChange }) {
  const shared = { label: field.label, help: field.help };
  switch (field.kind) {
    case "number":
      return (
        <NumberField
          {...shared}
          required={field.required}
          value={typeof value === "number" ? value : null}
          onChange={(number) => onChange(field.name, number)}
        />
      );
    case "boolean":
      return <Checkbox {...shared} checked={value === true} onChange={(checked) => onChange(field.name, checked)} />;
    case "choice":
      return (
        <Select
          {...shared}
          required={field.required}
          value={field.choices.includes(value) ? value : ""}
          options={choiceOptions(field.choices, value)}
          onChange={(choice) => onChange(field.name, choice)}
        />
      );
    case "json":
      return (
        <TextField
          {...shared}
          required={field.required}
          multiline
          monospace
          value={typeof value === "string" ? value : ""}
          onChange={(text) => onChange(field.name, text)}
        />
      );
    default:
      return (
        <TextField
          {...shared}
          required={field.required}
          value={typeof value === "string" ? value : ""}
          onChange={(text) => onChange(field.name, text)}
        />
      );
  }
}

/**
 * The choices as Select options, led by an empty "Choose…" option while the value is not one of the choices: no
 * value, "", or a stale value the choices no longer hold. The browser would otherwise select the first choice, so the
 * form would show a value it doesn't report. The empty value is what makes a native required <select> invalid.
 * @param {string[]} choices
 * @param {any} value
 * @returns {import("./Select.jsx").SelectOption[]}
 */
function choiceOptions(choices, value) {
  const options = choices.map((choice) => ({ value: choice, label: choice }));
  if (choices.includes(value)) {
    return options;
  }
  return [{ value: "", label: NO_CHOICE_LABEL }, ...options];
}
