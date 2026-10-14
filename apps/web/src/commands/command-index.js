// The command registry as the palette reads it (layout frontend-and-plugins.md 2.5): the descriptors ListCommands
// returned, found by id and read for their arguments and kind. Plain functions over the descriptors. The daemon says what
// a command is; these functions only read what it said.

import { CommandKind } from "@rastai/proto";

/** @typedef {import("@rastai/proto").CommandDescriptor} CommandDescriptor */

/** @type {Object<string, string>} keyed by the CommandKind value, as text */
const KIND_WORDS = {
  [String(CommandKind.SYNC)]: "Runs now",
  [String(CommandKind.JOB)]: "Starts a job",
  [String(CommandKind.STREAM)]: "Streams its output",
};

/**
 * The descriptor with this id, or null when the daemon didn't list it.
 * @param {CommandDescriptor[]} commands
 * @param {string} commandId
 * @returns {CommandDescriptor | null}
 */
export function findCommand(commands, commandId) {
  return commands.find((command) => command.commandId === commandId) ?? null;
}

/**
 * What a command's kind means to the person reading the detail pane.
 * @param {number} kind   a CommandKind value
 * @returns {string}
 */
export function kindWord(kind) {
  return KIND_WORDS[String(kind)] ?? "Runs";
}

/**
 * The names of a command's arguments, in the schema's order.
 * @param {string} argumentsSchemaJson
 * @returns {string[]}
 */
export function argumentNames(argumentsSchemaJson) {
  return Object.keys(parseSchema(argumentsSchemaJson).properties);
}

/**
 * The arguments the schema requires, in the schema's order. A command with none runs from the palette at once.
 * @param {string} argumentsSchemaJson
 * @returns {string[]}
 */
export function requiredArgumentNames(argumentsSchemaJson) {
  const schema = parseSchema(argumentsSchemaJson);
  return schema.required.filter((/** @type {string} */ name) => Object.hasOwn(schema.properties, name));
}

/**
 * The parts of an arguments JSON Schema the palette reads. Text that isn't an object schema reads as no arguments.
 * @param {string} argumentsSchemaJson
 * @returns {{ properties: Object<string, any>, required: string[] }}
 */
function parseSchema(argumentsSchemaJson) {
  try {
    const schema = JSON.parse(argumentsSchemaJson);
    const properties = schema?.properties;
    const required = Array.isArray(schema?.required) ? schema.required : [];
    return {
      properties: properties && typeof properties === "object" ? properties : {},
      required: required.filter((/** @type {any} */ name) => typeof name === "string"),
    };
  } catch {
    return { properties: {}, required: [] };
  }
}
