// RRS-U Diagnostics for transport and session failures (layout frontend-and-plugins.md 3.1; contracts 10).
// The client may not import @rastai/catalog (contracts 2.3), so the messages of the few codes it raises are kept
// here; diagnostics.test.js compares each one with catalog/errors/RRS-U.json, so the two can't drift.

import { DiagnosticSchema, Severity } from "@rastai/proto";

import {
  CODE_DAEMON_UNREACHABLE,
  CODE_LOGIN_CODE_REJECTED,
  CODE_SIGNED_OUT,
  CODE_UPLOAD_TOO_LARGE,
  SESSION_REFUSED_CODES,
} from "./constants.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * The catalog's message for each code the client raises, with its {snake_case} placeholders.
 * @type {Record<string, string>}
 */
export const CLIENT_MESSAGES = {
  [CODE_DAEMON_UNREACHABLE]: "The daemon at {address} is not reachable.",
  [CODE_LOGIN_CODE_REJECTED]: "The login code was rejected or expired.",
  [CODE_SIGNED_OUT]: "The session expired or was rejected; you are signed out.",
  [CODE_UPLOAD_TOO_LARGE]: "{file_name} is too large to upload from the browser.",
};
// Protobuf-es messages are plain objects; this field names the message, as `create(DiagnosticSchema)` would.
const DIAGNOSTIC_TYPE_NAME = "rastai.common.v1.Diagnostic";
const PLACEHOLDER = /\{([a-z_]+)\}/g;
// rastai-diag writes the severity as one of these words on the plain HTTP paths.
/** @type {Record<string, Severity>} */
const SEVERITY_BY_NAME = { error: Severity.ERROR, warning: Severity.WARNING, info: Severity.INFO };

/**
 * A Diagnostic for a failure the client found itself. Every code it raises is an error. The UI shows the
 * catalog's plain text and fix by code (DiagnosticView), so only the filled-in message is set here.
 * @param {string} code                     one of CLIENT_MESSAGES' codes
 * @param {Record<string, string>} [values] the message's placeholders, e.g. { address: "http://127.0.0.1:47820" }
 * @returns {Diagnostic}
 */
export function clientDiagnostic(code, values) {
  return {
    $typeName: DIAGNOSTIC_TYPE_NAME,
    code,
    severity: Severity.ERROR,
    message: fillPlaceholders(CLIENT_MESSAGES[code] ?? code, values ?? {}),
    path: "",
    fixText: "",
    fixCommandId: "",
    fixArgumentsJson: "",
  };
}

/**
 * Replaces each {name} with its value. A placeholder without a value stays as it is, so a missing value shows.
 * @param {string} template
 * @param {Record<string, string>} values
 * @returns {string}
 */
function fillPlaceholders(template, values) {
  return template.replace(PLACEHOLDER, (placeholder, name) => values[name] ?? placeholder);
}

/**
 * The daemon's own Diagnostic on a failed call: a gate refusal, or the error that ended a stream. Null when the
 * call never reached the daemon or the answer carried none (a dropped connection, a proxy's error page).
 * connect-es puts the Connect error's details on the error it throws; the daemon sends exactly one, the Diagnostic
 * (control-plane layout 11.3).
 * @param {any} error   what a connect-es call threw
 * @returns {Diagnostic | null}
 */
export function daemonDiagnosticOf(error) {
  if (error === null || typeof error !== "object" || typeof error.findDetails !== "function") {
    return null;
  }
  const details = error.findDetails(DiagnosticSchema);
  return details.length > 0 ? details[0] : null;
}

/**
 * The daemon's refusal on a plain HTTP path (blobs, login): the Diagnostic's fields as JSON with proto field names
 * and the severity as a lower-case word (rastai-diag's render_json). Null when the body isn't one.
 * @param {any} json
 * @returns {Diagnostic | null}
 */
export function diagnosticFromRefusalJson(json) {
  if (json === null || typeof json !== "object" || typeof json.code !== "string") {
    return null;
  }
  return {
    $typeName: DIAGNOSTIC_TYPE_NAME,
    code: json.code,
    severity: SEVERITY_BY_NAME[json.severity] ?? Severity.ERROR,
    message: String(json.message ?? ""),
    path: String(json.path ?? ""),
    fixText: String(json.fix_text ?? ""),
    fixCommandId: String(json.fix_command_id ?? ""),
    fixArgumentsJson: String(json.fix_arguments_json ?? ""),
  };
}

/**
 * True for the daemon's refusals that mean the session or token is gone: retrying with it can't work.
 * @param {Diagnostic} diagnostic
 * @returns {boolean}
 */
export function isSessionRefusal(diagnostic) {
  return SESSION_REFUSED_CODES.includes(diagnostic.code);
}

/**
 * What a caller is told about a failed request: RRS-U0001 when the daemon couldn't be reached (no refusal of its
 * own), RRS-U0003 when it refused the session, and otherwise the daemon's refusal as it sent it (it already has a
 * catalog code).
 * @param {Diagnostic | null} refusal   the daemon's Diagnostic, if its answer carried one
 * @param {string} baseUrl
 * @returns {Diagnostic}
 */
export function failureDiagnostic(refusal, baseUrl) {
  if (refusal === null) {
    return clientDiagnostic(CODE_DAEMON_UNREACHABLE, { address: baseUrl });
  }
  if (isSessionRefusal(refusal)) {
    return clientDiagnostic(CODE_SIGNED_OUT);
  }
  return refusal;
}

/**
 * failureDiagnostic for what a connect-es call threw.
 * @param {any} error
 * @param {string} baseUrl
 * @returns {Diagnostic}
 */
export function callFailureDiagnostic(error, baseUrl) {
  return failureDiagnostic(daemonDiagnosticOf(error), baseUrl);
}
