// Copy as (layout frontend-and-plugins.md 2.5 step 7): one command call written four ways, CLI, Python, JavaScript and
// MCP. The rules are the ones fixtures/command-golden/README.md states, and copy-as.test.js checks every line of the
// golden file against them, so the UI, the CLI's parser and the naming rules can't drift apart. Pure, so it is tested
// in Node.

import { secretArgumentNames } from "./command-index.js";

// The CLI words of an id are its segments with "_" written as "-"; the MCP tool name writes each "." as "__".
const ID_SEPARATOR = ".";
const CLI_PREFIX = "rastai";
const CLI_FLAG_PREFIX = "--";
const CLI_NEGATED_FLAG_PREFIX = "--no-";
const CLI_WORD_SEPARATOR = "-";
const SNAKE_WORD_SEPARATOR = "_";
const MCP_SEPARATOR = "__";
// Text of only these characters needs no shell quotes (fixtures/command-golden/README.md).
const SHELL_SAFE_TEXT = /^[A-Za-z0-9_./:@=,+-]+$/;
const PYTHON_CONNECT = "rastai.connect()";
const JAVASCRIPT_CLIENT = "client";
const JS_IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const MCP_INDENT = 2;

/**
 * @typedef {Object<string, any>} CallArguments   the arguments of one call, in the order they are written
 */

/**
 * The CLI line: `rastai <words> --<flag> <value> ...`. Flags only, never positions; a true boolean is its flag alone, a
 * false one its "--no-" flag, and a list repeats the flag once per element.
 * @param {string} commandId
 * @param {CallArguments} args
 * @returns {string}
 */
export function formatCli(commandId, args) {
  const words = commandId.split(ID_SEPARATOR).map(cliWord);
  const flags = Object.entries(args).flatMap(([field, value]) => cliFlags(field, value));
  return [CLI_PREFIX, ...words, ...flags].join(" ");
}

/**
 * The Python line: the SDK client's execute call with the arguments as a dict.
 * @param {string} commandId
 * @param {CallArguments} args
 * @returns {string}
 */
export function formatPython(commandId, args) {
  return `${PYTHON_CONNECT}.execute(${JSON.stringify(commandId)}, ${pythonValue(args)})`;
}

/**
 * The JavaScript line: the client's execute call with the arguments as an object.
 * @param {string} commandId
 * @param {CallArguments} args
 * @returns {string}
 */
export function formatJavaScript(commandId, args) {
  return `await ${JAVASCRIPT_CLIENT}.execute(${JSON.stringify(commandId)}, ${javaScriptValue(args)})`;
}

/**
 * The MCP tools/call parameters: the tool name and the arguments, as indented JSON.
 * @param {string} commandId
 * @param {CallArguments} args
 * @returns {string}
 */
export function formatMcp(commandId, args) {
  const name = commandId.split(ID_SEPARATOR).join(MCP_SEPARATOR);
  return JSON.stringify({ name, arguments: args }, null, MCP_INDENT);
}

/**
 * The four formats of one call, keyed by the names the palette shows (cli, python, javascript, mcp). This is the one
 * place a call is written for the UI, so the secret arguments are left out here, in every format, before any of them
 * is written.
 * @param {string} commandId
 * @param {CallArguments} args
 * @param {string} argumentsSchemaJson   the command's arguments_schema_json, which marks the secret fields; "" when none
 * @returns {Object<string, string>}
 */
export function copyAsFormats(commandId, args, argumentsSchemaJson) {
  const shown = withoutSecrets(args, argumentsSchemaJson);
  return {
    cli: formatCli(commandId, shown),
    python: formatPython(commandId, shown),
    javascript: formatJavaScript(commandId, shown),
    mcp: formatMcp(commandId, shown),
  };
}

/**
 * The arguments without the secret ones. A placeholder would not do: pasted into a call, "<secret>" would be stored as
 * the credential (secret.set would keep that text as the value). Left out, the copied call is missing the argument and
 * the daemon refuses it instead.
 * @param {CallArguments} args
 * @param {string} argumentsSchemaJson
 * @returns {CallArguments}
 */
function withoutSecrets(args, argumentsSchemaJson) {
  const secretNames = secretArgumentNames(argumentsSchemaJson);
  /** @type {CallArguments} */
  const shown = {};
  for (const [name, value] of Object.entries(args)) {
    if (!secretNames.includes(name)) {
      shown[name] = value;
    }
  }
  return shown;
}

/**
 * "eval.suite.add_conversation" words: each segment with "_" written as "-".
 * @param {string} segment
 * @returns {string}
 */
function cliWord(segment) {
  return segment.split(SNAKE_WORD_SEPARATOR).join(CLI_WORD_SEPARATOR);
}

/**
 * The flags of one argument. An absent value writes no flag at all.
 * @param {string} field
 * @param {any} value
 * @returns {string[]}
 */
function cliFlags(field, value) {
  const flag = CLI_FLAG_PREFIX + cliWord(field);
  if (value === undefined || value === null) {
    return [];
  }
  if (typeof value === "boolean") {
    return [value ? flag : CLI_NEGATED_FLAG_PREFIX + cliWord(field)];
  }
  if (Array.isArray(value)) {
    return value.flatMap((element) => [flag, shellText(cliText(element))]);
  }
  return [flag, shellText(cliText(value))];
}

/**
 * The text a CLI flag takes for a value: numbers and text as they are, a nested value as JSON.
 * @param {any} value
 * @returns {string}
 */
function cliText(value) {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
}

/**
 * Text as a POSIX shell reads it: left bare when it has only safe characters, else in single quotes, with each quote
 * written as '\''.
 * @param {string} text
 * @returns {string}
 */
function shellText(text) {
  if (SHELL_SAFE_TEXT.test(text)) {
    return text;
  }
  return `'${text.split("'").join("'\\''")}'`;
}

/**
 * A Python literal: True and False, None, a double-quoted string, a list, or a dict with the keys in order.
 * @param {any} value
 * @returns {string}
 */
function pythonValue(value) {
  if (value === null || value === undefined) {
    return "None";
  }
  if (typeof value === "boolean") {
    return value ? "True" : "False";
  }
  if (Array.isArray(value)) {
    return `[${value.map(pythonValue).join(", ")}]`;
  }
  if (typeof value === "object") {
    const entries = Object.entries(value).map(([key, item]) => `${JSON.stringify(key)}: ${pythonValue(item)}`);
    return `{${entries.join(", ")}}`;
  }
  return typeof value === "string" ? JSON.stringify(value) : String(value);
}

/**
 * A JavaScript literal: true and false, null, a double-quoted string, an array, or an object with the keys in order,
 * each key bare when it is an identifier.
 * @param {any} value
 * @returns {string}
 */
function javaScriptValue(value) {
  if (value === null || value === undefined) {
    return "null";
  }
  if (Array.isArray(value)) {
    return `[${value.map(javaScriptValue).join(", ")}]`;
  }
  if (typeof value === "object") {
    const entries = Object.entries(value).map(([key, item]) => `${javaScriptKey(key)}: ${javaScriptValue(item)}`);
    return entries.length === 0 ? "{}" : `{ ${entries.join(", ")} }`;
  }
  return JSON.stringify(value);
}

/**
 * @param {string} key
 * @returns {string}
 */
function javaScriptKey(key) {
  return JS_IDENTIFIER.test(key) ? key : JSON.stringify(key);
}
