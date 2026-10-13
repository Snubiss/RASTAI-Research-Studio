// One Diagnostic, the way every error in the app is shown (layout frontend-and-plugins.md 2.4 step 6 and 2.7).
// Basic: the catalog's plain text and a short fix button, with the catalog's fix sentence beside it so the user
// knows what the button will do (a fix with no command shows the sentence alone). Advanced adds the code, path and
// full message. The severity color is a locked token (--rr-locked-severity-*), so no theme can make an error look
// like info.

import { Button } from "../components/Button.jsx";
import { useMode } from "../mode/use-mode.js";
import { diagnosticText } from "./catalog-lookup.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

const SEVERITY_LABELS = { error: "Error", warning: "Warning", info: "Note" };
// The fix button stays short; the catalog's fix_text is a whole sentence, so it goes beside the button.
const DEFAULT_FIX_LABEL = "Fix it";

/**
 * @param {object} props
 * @param {Diagnostic} props.diagnostic
 * @param {((commandId: string, argumentsJson: string) => void) | undefined} [props.onFix]
 *   runs the diagnostic's fix. Without it there is no button, only the fix sentence: a button that does nothing would lie.
 * @param {string} [props.fixLabel]   the fix button's text
 */
export function DiagnosticView({ diagnostic, onFix, fixLabel = DEFAULT_FIX_LABEL }) {
  const mode = useMode();
  const text = diagnosticText(diagnostic);
  const hasFix = diagnostic.fixCommandId !== "" && onFix !== undefined;
  return (
    <div
      className={`rr-diagnostic rr-diagnostic--${text.severity}`}
      role={text.severity === "error" ? "alert" : "status"}
    >
      <p className="rr-diagnostic__summary">
        <span className="rr-diagnostic__severity">{SEVERITY_LABELS[text.severity]}</span>{" "}
        <span className="rr-diagnostic__basic">{text.basicText}</span>
      </p>
      {mode === "advanced" ? <AdvancedDetails diagnostic={diagnostic} title={text.title} /> : null}
      {hasFix || text.fixText ? (
        <div className="rr-diagnostic__fix">
          {hasFix ? (
            <Button variant="primary" onClick={() => onFix?.(diagnostic.fixCommandId, diagnostic.fixArgumentsJson)}>
              {fixLabel}
            </Button>
          ) : null}
          {text.fixText ? <span className="rr-diagnostic__fix-text">{text.fixText}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * What Advanced adds after the Basic text: the code and title, where it happened and the full message.
 * @param {object} props
 * @param {Diagnostic} props.diagnostic
 * @param {string} props.title
 */
function AdvancedDetails({ diagnostic, title }) {
  return (
    <dl className="rr-diagnostic__details">
      <dt>Code</dt>
      <dd>
        <code>{diagnostic.code}</code> {title}
      </dd>
      {diagnostic.path ? (
        <>
          <dt>Where</dt>
          <dd>
            <code>{diagnostic.path}</code>
          </dd>
        </>
      ) : null}
      <dt>Message</dt>
      <dd>{diagnostic.message}</dd>
    </dl>
  );
}
