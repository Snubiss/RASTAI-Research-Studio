// The few Diagnostics the SDK raises itself, built from the generated catalog (contracts 10), so the message is the
// catalog's with its placeholders filled and DiagnosticView finds the plain text and fix by code.

import { CATALOG } from "@rastai/catalog";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/** A command id nothing answers to: the mock host's reply for a command without a canned outcome. */
export const CODE_UNKNOWN_COMMAND = "RRS-C0001";
/** The daemon can't be reached, or a call got no answer (catalog RRS-U0001). */
export const CODE_DAEMON_UNREACHABLE = "RRS-U0001";
/** A plugin called a command outside its commands:[scopes] permission (contracts 6.3). */
export const CODE_SCOPE_NOT_GRANTED = "RRS-C1005";

// Protobuf-es messages are plain objects; this field names the message, as `create(DiagnosticSchema)` would.
const DIAGNOSTIC_TYPE_NAME = "rastai.common.v1.Diagnostic";
// Diagnostic.severity is the protobuf enum rastai.common.v1.Severity; its numbers never change. Writing them here
// keeps the protobuf runtime out of plugin bundles (layout 7.3: protobuf-es is reached only through @rastai/client).
/** @type {Object<string, number>} */
const SEVERITY_WIRE_VALUES = { error: 1, warning: 2, info: 3 };
const PLACEHOLDER = /\{([a-z_]+)\}/g;

/**
 * A Diagnostic for a catalog code, with the catalog's message and severity.
 * @param {string} code
 * @param {Object<string, string>} values   the message's placeholders, e.g. { command_id: "model.load" }
 * @returns {Diagnostic}
 */
export function pluginDiagnostic(code, values) {
  const entry = CATALOG.find((candidate) => candidate.code === code);
  const template = entry ? entry.message : code;
  return {
    $typeName: DIAGNOSTIC_TYPE_NAME,
    code,
    severity: SEVERITY_WIRE_VALUES[entry ? entry.severity : "error"],
    message: template.replace(PLACEHOLDER, (placeholder, name) => values[name] ?? placeholder),
    path: "",
    fixText: "",
    fixCommandId: "",
    fixArgumentsJson: "",
  };
}

/**
 * The Diagnostic for a call whose execute rejected instead of answering: nothing came back from the daemon, which
 * is what RRS-U0001 says. The thrown message is kept for Advanced mode, which shows the full text.
 * @param {unknown} error
 * @returns {Diagnostic}
 */
export function callFailedDiagnostic(error) {
  const message = error instanceof Error && error.message !== "" ? error.message : "The call did not complete.";
  return { ...pluginDiagnostic(CODE_DAEMON_UNREACHABLE, { address: "" }), message };
}

/**
 * The outcome of a call that was refused before it reached the daemon.
 * @param {Diagnostic} diagnostic
 * @returns {import("@rastai/client").ExecuteOutcome}
 */
export function refusedOutcome(diagnostic) {
  return { ok: false, result: null, jobId: null, diagnostics: [diagnostic], journalId: null };
}
