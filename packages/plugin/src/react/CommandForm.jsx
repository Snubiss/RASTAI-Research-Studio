// A form for any command, drawn from its arguments JSON Schema (layout frontend-and-plugins.md 7.1): the palette's
// "run with arguments", a plugin's settings page, a section's advanced controls. The fields are @rastai/ui's
// FieldForm; this component only turns the schema into fields, the values into arguments, and runs the command.

import { useMemo, useState } from "react";

import { Button, FieldForm } from "@rastai/ui";

import { fieldValuesToArgs, schemaToFields } from "../commands/schema-to-fields.js";
import { DiagnosticList } from "./DiagnosticList.jsx";
import { useCommand } from "./use-command.js";

/** @typedef {import("../host-api.js").ExecuteOutcome} ExecuteOutcome */

const DEFAULT_SUBMIT_LABEL = "Run";

/**
 * @param {object} props
 * @param {string} props.commandId
 * @param {string} props.argumentsSchemaJson   the CommandDescriptor's arguments_schema_json
 * @param {string} [props.submitLabel]
 * @param {(outcome: ExecuteOutcome) => void} [props.onOutcome]   called after every run, ok or not
 */
export function CommandForm({ commandId, argumentsSchemaJson, submitLabel = DEFAULT_SUBMIT_LABEL, onOutcome }) {
  const fields = useMemo(() => schemaToFields(argumentsSchemaJson), [argumentsSchemaJson]);
  const [values, setValues] = useState(/** @type {Object<string, any>} */ ({}));
  const command = useCommand(commandId);

  /** @param {import("react").FormEvent} event */
  async function submit(event) {
    event.preventDefault();
    const outcome = await command.run(fieldValuesToArgs(fields, values));
    onOutcome?.(outcome);
  }

  return (
    <form onSubmit={submit}>
      <FieldForm fields={fields} values={values} onChange={setValues} />
      <Button type="submit" variant="primary" disabled={command.running}>
        {submitLabel}
      </Button>
      {command.outcome ? <DiagnosticList diagnostics={command.outcome.diagnostics} /> : null}
    </form>
  );
}
