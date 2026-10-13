// The First Run checklist (design 6.2): hardware, the Python environment, Hugging Face sign-in, a model download, the
// SAE weights. Each step is a command. The steps this version can't run show "not available yet" with RRS-C0010, never
// a made-up state. The list of commands comes from command.list, through the section's host. Advanced adds the command
// id of each step.

import { DiagnosticView, useMode } from "@rastai/ui";
import { useQuery } from "@rastai/plugin";

import { notAvailableDiagnostic } from "../not-available.js";

/**
 * The steps, in order. The command id is the one the step will run; the daemon's registry says whether it exists.
 * @type {{ stepId: string, title: string, commandId: string }[]}
 */
export const FIRST_RUN_STEPS = [
  { stepId: "hardware", title: "Check the hardware", commandId: "system.hardware" },
  { stepId: "environment", title: "Set up the Python environment", commandId: "system.env.install" },
  { stepId: "hf_sign_in", title: "Sign in to Hugging Face and accept the model license", commandId: "hf.login" },
  { stepId: "model", title: "Download a model", commandId: "model.download" },
  { stepId: "sae", title: "Load the SAE weights for three layers", commandId: "psychology.sae.load" },
];

/**
 * Whether the daemon's command list has a command. command.list answers in the protobuf JSON names (command_id).
 * @param {{ command_id: string }[]} commands
 * @param {string} commandId
 * @returns {boolean}
 */
function commandListed(commands, commandId) {
  return commands.some((command) => command.command_id === commandId);
}

export function FirstRunChecklist() {
  const mode = useMode();
  const listing = useQuery("command.list", {}, ["plugin.state_changed"]);
  return (
    <section className="rr-home-panel" aria-labelledby="first-run-title">
      <h2 id="first-run-title" className="rr-home-panel__title">
        First run
      </h2>
      {listing.status === "error" ? (
        <DiagnosticView diagnostic={listing.diagnostics[0]} />
      ) : (
        <ol className="rr-first-run">
          {FIRST_RUN_STEPS.map((step) => (
            <FirstRunStep key={step.stepId} step={step} mode={mode} listing={listing} />
          ))}
        </ol>
      )}
    </section>
  );
}

/**
 * One step: ready when its command is listed, else not available yet. While the list loads it says so.
 * @param {object} props
 * @param {{ stepId: string, title: string, commandId: string }} props.step
 * @param {"basic" | "advanced"} props.mode
 * @param {import("@rastai/plugin").QueryState} props.listing
 */
function FirstRunStep({ step, mode, listing }) {
  const commands = listing.status === "ready" ? listing.data.commands : [];
  let state = <span className="rr-first-run__state">Checking…</span>;
  if (listing.status === "ready") {
    state = commandListed(commands, step.commandId) ? (
      <span className="rr-first-run__state">Ready to run</span>
    ) : (
      <DiagnosticView diagnostic={notAvailableDiagnostic(step.commandId)} />
    );
  }
  return (
    <li className="rr-first-run__step" data-step={step.stepId}>
      <span className="rr-first-run__title">{step.title}</span>
      {state}
      {mode === "advanced" ? <code className="rr-first-run__command">{step.commandId}</code> : null}
    </li>
  );
}
