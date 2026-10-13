// CommandForm's fields from real command schemas (layout frontend-and-plugins.md 7.5): the arguments part of the
// checked-in schemas/commands/ files is what the daemon sends as arguments_schema_json (both come from
// rastai-formats' json_schema_for).

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { fieldValuesToArgs, schemaToFields } from "./schema-to-fields.js";

const COMMAND_SCHEMAS = new URL("../../../../schemas/commands/", import.meta.url);

/**
 * The arguments schema of a checked-in command schema, as the daemon would send it.
 * @param {string} commandId
 * @returns {string}
 */
function argumentsSchemaJson(commandId) {
  const file = new URL(`rastai.command.${commandId}/1.json`, COMMAND_SCHEMAS);
  return JSON.stringify(JSON.parse(readFileSync(file, "utf8")).$defs.arguments);
}

describe("schemaToFields", () => {
  test("a string, an enum and any-JSON value become text, choice and json fields", () => {
    expect(schemaToFields(argumentsSchemaJson("settings.set"))).toEqual([
      { name: "key", label: "Key", kind: "text", required: true, help: "A dotted key, such as ui.theme.", choices: [] },
      {
        name: "level",
        label: "Level",
        kind: "choice",
        required: false,
        help: "Which settings.json to write: user (the default) or project, the open project's.",
        choices: ["user", "project"],
      },
      {
        name: "value",
        label: "Value",
        kind: "json",
        required: true,
        help: "Any JSON value the key allows.",
        choices: [],
      },
    ]);
  });

  test("booleans are checkboxes and multi-word names read as words", () => {
    const fields = schemaToFields(argumentsSchemaJson("command.list"));
    expect(fields.map((field) => [field.name, field.label, field.kind, field.required])).toEqual([
      ["basic_only", "Basic only", "boolean", false],
      ["include_experimental", "Include experimental", "boolean", false],
      ["section", "Section", "text", false],
    ]);
  });

  test("integers and numbers are number fields; lists, maps and nested types are json", () => {
    const schema = {
      type: "object",
      properties: {
        max_steps: { type: "integer" },
        learning_rate: { type: "number" },
        tags: { type: "array", items: { type: "string" } },
        lora: { $ref: "#/$defs/LoraSettings" },
      },
      required: ["max_steps"],
    };
    expect(schemaToFields(JSON.stringify(schema)).map((field) => [field.name, field.kind, field.required])).toEqual([
      ["max_steps", "number", true],
      ["learning_rate", "number", false],
      ["tags", "json", false],
      ["lora", "json", false],
    ]);
  });

  test("text that isn't an object schema gives no fields", () => {
    expect(schemaToFields("")).toEqual([]);
    expect(schemaToFields("[1, 2]")).toEqual([]);
    expect(schemaToFields('{"type":"object"}')).toEqual([]);
  });
});

describe("fieldValuesToArgs", () => {
  const fields = schemaToFields(argumentsSchemaJson("settings.set"));

  test("leaves empty fields out and parses JSON fields", () => {
    expect(fieldValuesToArgs(fields, { key: "ui.density", level: "", value: '"compact"' })).toEqual({
      key: "ui.density",
      value: "compact",
    });
  });

  test("sends JSON text that doesn't parse as the text, for the daemon to report", () => {
    expect(fieldValuesToArgs(fields, { key: "ui.theme", value: "dark" })).toEqual({ key: "ui.theme", value: "dark" });
  });

  test("keeps false and 0, which are values, not empty fields", () => {
    const toggles = schemaToFields(
      JSON.stringify({ properties: { basic_only: { type: "boolean" }, top_k: { type: "integer" } } }),
    );
    expect(fieldValuesToArgs(toggles, { basic_only: false, top_k: 0 })).toEqual({ basic_only: false, top_k: 0 });
  });
});
