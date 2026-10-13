// Between a command's arguments JSON Schema and the form CommandForm draws (layout frontend-and-plugins.md 7.1).
// Only the subset rastai-formats writes is read (rastai-formats/src/describe/json_schema.rs): an object of named
// properties, each a string, an enum of strings, an integer, a number, a boolean, or anything else (a list, a map,
// a nested type, any JSON), which the form edits as JSON text. Validation stays with the daemon: a value the schema
// doesn't allow comes back as the daemon's diagnostic, with its catalog code.

/** @typedef {import("@rastai/ui").FieldSpec} FieldSpec */

/** @type {Object<string, "text" | "number" | "boolean">} */
const KIND_BY_SCHEMA_TYPE = { string: "text", integer: "number", number: "number", boolean: "boolean" };

/**
 * The form fields for a command's arguments_schema_json, in the schema's property order.
 * @param {string} argumentsSchemaJson
 * @returns {FieldSpec[]} empty when the text isn't an object schema
 */
export function schemaToFields(argumentsSchemaJson) {
  const schema = parseObject(argumentsSchemaJson);
  const properties = schema?.properties;
  if (!properties || typeof properties !== "object") {
    return [];
  }
  const required = Array.isArray(schema.required) ? schema.required : [];
  return Object.entries(properties).map(([name, property]) => fieldOf(name, property, required.includes(name)));
}

/**
 * The command arguments for the form's values. Empty fields are left out, so the daemon applies its defaults; a
 * JSON field whose text doesn't parse is sent as the text, and the daemon's diagnostic says what is wrong with it.
 * @param {FieldSpec[]} fields
 * @param {Object<string, any>} values
 * @returns {Object<string, any>}
 */
export function fieldValuesToArgs(fields, values) {
  /** @type {Object<string, any>} */
  const args = {};
  for (const field of fields) {
    const value = values[field.name];
    if (value === undefined || value === null || value === "") {
      continue;
    }
    args[field.name] = field.kind === "json" ? parseJsonOrText(value) : value;
  }
  return args;
}

/**
 * @param {string} name
 * @param {any} property   one property's schema
 * @param {boolean} required
 * @returns {FieldSpec}
 */
function fieldOf(name, property, required) {
  const label = typeof property?.title === "string" ? property.title : labelOf(name);
  const help = typeof property?.description === "string" ? property.description : "";
  if (Array.isArray(property?.enum) && property.enum.every((/** @type {any} */ choice) => typeof choice === "string")) {
    return { name, label, kind: "choice", required, help, choices: property.enum };
  }
  const kind = KIND_BY_SCHEMA_TYPE[property?.type] ?? "json";
  return { name, label, kind, required, help, choices: [] };
}

/**
 * "spec_path" -> "Spec path".
 * @param {string} name
 * @returns {string}
 */
function labelOf(name) {
  const words = name.replaceAll("_", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * @param {string} text
 * @returns {any} the parsed object, or null when the text isn't a JSON object
 */
function parseObject(text) {
  try {
    const value = JSON.parse(text);
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/**
 * @param {string} text
 * @returns {any}
 */
function parseJsonOrText(text) {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}
