// The shared table every CommandExecutor must pass (layout frontend-and-plugins.md 3.5, 7.5): DaemonClient here,
// each PluginHost in @rastai/plugin's tests. It lives beside CommandExecutor so those tests import it downward
// ("@rastai/client/testing"). Plain data and one plain function; each test suite wires the replies to its own
// fake daemon and checks the request the daemon got and the outcome execute resolved to.

/** @typedef {import("../types.js").ExecuteOptions} ExecuteOptions */
/** @typedef {import("../types.js").ExecuteOutcome} ExecuteOutcome */

/**
 * What the daemon side does in one case.
 * - "answer": Execute answers with `response`, the ExecuteResponse fields in protobuf-es's camelCase.
 * - "unreachable": nothing answers; the executor must resolve to RRS-U0001.
 * - "session_refused": the daemon refuses the credential (RRS-C1002); the executor must resolve to RRS-U0003.
 * @typedef {object} ContractReply
 * @property {"answer" | "unreachable" | "session_refused"} kind
 * @property {object} [response]
 */

/**
 * The ExecuteRequest fields the daemon must receive, with the arguments parsed back from arguments_json. Null when
 * no request reaches the Execute handler.
 * @typedef {object} ContractRequest
 * @property {string} commandId
 * @property {object} args
 * @property {string} projectId
 * @property {boolean} dryRun
 * @property {string} actingPluginId
 */

/**
 * An ExecuteOutcome with its diagnostics cut down to their codes, so outcomes compare with toEqual.
 * @typedef {object} OutcomeSummary
 * @property {boolean} ok
 * @property {any} result
 * @property {string | null} jobId
 * @property {string | null} journalId
 * @property {string[]} diagnosticCodes
 */

/**
 * @typedef {object} ExecutorContractCase
 * @property {string} name
 * @property {string} commandId
 * @property {object} args
 * @property {ExecuteOptions} [options]
 * @property {ContractReply} reply
 * @property {ContractRequest | null} request
 * @property {OutcomeSummary} outcome
 */

const NO_OPTIONS = { projectId: "", dryRun: false, actingPluginId: "" };

/** @type {ExecutorContractCase[]} */
export const EXECUTOR_CONTRACT_CASES = [
  {
    name: "a sync command's result_json comes back parsed",
    commandId: "settings.get",
    args: { key: "ui.theme" },
    reply: {
      kind: "answer",
      response: {
        ok: true,
        resultJson: '{"key":"ui.theme","value":"dark"}',
        journalId: "jrn_01JA2B3C4D5E6F7G8H9J0K1M2R",
      },
    },
    request: { commandId: "settings.get", args: { key: "ui.theme" }, ...NO_OPTIONS },
    outcome: {
      ok: true,
      result: { key: "ui.theme", value: "dark" },
      jobId: null,
      journalId: "jrn_01JA2B3C4D5E6F7G8H9J0K1M2R",
      diagnosticCodes: [],
    },
  },
  {
    name: "a job command returns its job id",
    commandId: "train.start",
    args: { spec_path: "designs/textcnn.rrsmodel" },
    reply: {
      kind: "answer",
      response: { ok: true, resultJson: '{"job_id":"job_01"}', jobId: "job_01", journalId: "jrn_02" },
    },
    request: { commandId: "train.start", args: { spec_path: "designs/textcnn.rrsmodel" }, ...NO_OPTIONS },
    outcome: { ok: true, result: { job_id: "job_01" }, jobId: "job_01", journalId: "jrn_02", diagnosticCodes: [] },
  },
  {
    name: "a command the daemon refuses is an outcome, not a thrown error",
    commandId: "no.such_command",
    args: {},
    reply: {
      kind: "answer",
      response: { ok: false, diagnostics: [{ code: "RRS-C0001", message: "Unknown command no.such_command." }] },
    },
    request: { commandId: "no.such_command", args: {}, ...NO_OPTIONS },
    outcome: { ok: false, result: null, jobId: null, journalId: null, diagnosticCodes: ["RRS-C0001"] },
  },
  {
    name: "warnings come back beside a success",
    commandId: "settings.set",
    args: { key: "ui.theme", value: "dark" },
    reply: {
      kind: "answer",
      response: {
        ok: true,
        resultJson: '{"key":"ui.theme","value":"dark","source":"user"}',
        diagnostics: [{ code: "RRS-E3001" }],
        journalId: "jrn_03",
      },
    },
    request: { commandId: "settings.set", args: { key: "ui.theme", value: "dark" }, ...NO_OPTIONS },
    outcome: {
      ok: true,
      result: { key: "ui.theme", value: "dark", source: "user" },
      jobId: null,
      journalId: "jrn_03",
      diagnosticCodes: ["RRS-E3001"],
    },
  },
  {
    name: "the project and dry-run options reach the daemon",
    commandId: "settings.set",
    args: { key: "ui.theme", value: "light", level: "project" },
    options: { projectId: "prj_01", dryRun: true },
    reply: { kind: "answer", response: { ok: true, resultJson: "" } },
    request: {
      commandId: "settings.set",
      args: { key: "ui.theme", value: "light", level: "project" },
      projectId: "prj_01",
      dryRun: true,
      actingPluginId: "",
    },
    outcome: { ok: true, result: null, jobId: null, journalId: null, diagnosticCodes: [] },
  },
  {
    name: "the acting plugin id reaches the daemon",
    commandId: "job.list",
    args: {},
    options: { actingPluginId: "psychology" },
    reply: { kind: "answer", response: { ok: true, resultJson: '{"jobs":[]}' } },
    request: { commandId: "job.list", args: {}, projectId: "", dryRun: false, actingPluginId: "psychology" },
    outcome: { ok: true, result: { jobs: [] }, jobId: null, journalId: null, diagnosticCodes: [] },
  },
  {
    name: "an unreachable daemon is RRS-U0001",
    commandId: "system.status",
    args: {},
    reply: { kind: "unreachable" },
    request: null,
    outcome: { ok: false, result: null, jobId: null, journalId: null, diagnosticCodes: ["RRS-U0001"] },
  },
  {
    name: "a refused session is RRS-U0003",
    commandId: "system.status",
    args: {},
    reply: { kind: "session_refused" },
    request: null,
    outcome: { ok: false, result: null, jobId: null, journalId: null, diagnosticCodes: ["RRS-U0003"] },
  },
];

/**
 * The outcome with each diagnostic cut down to its code, for comparing with a case's `outcome`.
 * @param {ExecuteOutcome} outcome
 * @returns {OutcomeSummary}
 */
export function summarizeOutcome(outcome) {
  return {
    ok: outcome.ok,
    result: outcome.result,
    jobId: outcome.jobId,
    journalId: outcome.journalId,
    diagnosticCodes: outcome.diagnostics.map((diagnostic) => diagnostic.code),
  };
}
