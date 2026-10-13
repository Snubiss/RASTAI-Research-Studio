// The catalog text DiagnosticView shows for a Diagnostic (layout frontend-and-plugins.md 2.7 and 4.1). A
// Diagnostic carries its message already filled in; the catalog's basic_text and fix_text may use the same
// {placeholders}, so their values are read back out of the message by matching it against the catalog's template.

import { CATALOG } from "@rastai/catalog";

/** @typedef {import("@rastai/catalog").CatalogEntry} CatalogEntry */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * What DiagnosticView shows, with every placeholder it could fill filled.
 * @typedef {object} DiagnosticText
 * @property {"error" | "warning" | "info"} severity
 * @property {string} title
 * @property {string} basicText
 * @property {string} fixText
 * @property {boolean} inCatalog   false for a code the catalog doesn't know (a plugin's own code, a newer daemon)
 */

const PLACEHOLDER = /\{([a-z_]+)\}/g;
const REGEX_SPECIAL = /[.*+?^${}()|[\]\\]/g;
// Diagnostic.severity is the protobuf enum (rastai.common.v1.Severity); its numbers never change.
/** @type {Object<number, "error" | "warning" | "info">} */
const SEVERITY_BY_WIRE_VALUE = { 1: "error", 2: "warning", 3: "info" };

/** @type {Map<string, CatalogEntry>} */
const ENTRIES_BY_CODE = new Map(CATALOG.map((entry) => [entry.code, entry]));

/**
 * @param {string} code
 * @returns {CatalogEntry | undefined}
 */
export function catalogEntry(code) {
  return ENTRIES_BY_CODE.get(code);
}

/**
 * @param {Diagnostic} diagnostic
 * @returns {DiagnosticText}
 */
export function diagnosticText(diagnostic) {
  const entry = catalogEntry(diagnostic.code);
  const severity = SEVERITY_BY_WIRE_VALUE[diagnostic.severity] ?? entry?.severity ?? "error";
  if (!entry) {
    return {
      severity,
      title: diagnostic.code,
      basicText: diagnostic.message,
      fixText: diagnostic.fixText,
      inCatalog: false,
    };
  }
  const values = placeholderValues(entry.message, diagnostic.message);
  return {
    severity,
    title: entry.title,
    basicText: fillPlaceholders(entry.basic_text, values),
    fixText: diagnostic.fixText || fillPlaceholders(entry.fix_text, values),
    inCatalog: true,
  };
}

/**
 * The values of a template's placeholders, read back from a message filled from it. A message that doesn't match
 * the template (it was written by an older catalog) gives no values.
 * @param {string} template   "The daemon at {address} is not reachable."
 * @param {string} message    "The daemon at http://127.0.0.1:47820 is not reachable."
 * @returns {Object<string, string>}
 */
export function placeholderValues(template, message) {
  const names = [...template.matchAll(PLACEHOLDER)].map((match) => match[1]);
  if (names.length === 0) {
    return {};
  }
  const literalParts = template.split(PLACEHOLDER).filter((_, index) => index % 2 === 0);
  const pattern = new RegExp(`^${literalParts.map(escapeRegex).join("(.*?)")}$`, "s");
  const match = pattern.exec(message);
  if (!match) {
    return {};
  }
  /** @type {Object<string, string>} */
  const values = {};
  names.forEach((name, index) => {
    values[name] = match[index + 1];
  });
  return values;
}

/**
 * Replaces each {name} with its value; a placeholder without a value stays as it is, so a missing value shows.
 * @param {string} template
 * @param {Object<string, string>} values
 * @returns {string}
 */
export function fillPlaceholders(template, values) {
  return template.replace(PLACEHOLDER, (whole, name) => values[name] ?? whole);
}

/**
 * @param {string} text
 * @returns {string}
 */
function escapeRegex(text) {
  return text.replace(REGEX_SPECIAL, "\\$&");
}
