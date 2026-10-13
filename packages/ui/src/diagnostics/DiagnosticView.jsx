// One Diagnostic, the way every error in the app is shown (layout frontend-and-plugins.md 2.4 step 6 and 2.7).
// Basic: the catalog's plain text and a fix button. Advanced adds the code, path, full message and fix text. The
// severity color is a locked token (--rr-locked-severity-*), so no theme can make an error look like info.

import { Button } from "../components/Button.jsx";
import { useMode } from "../mode/use-mode.js";
import { diagnosticText } from "./catalog-lookup.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

const SEVERITY_LABELS = { error: "Error", warning: "Warning", info: "Note" };

/**
 * @param {object} props
 * @param {Diagnostic} props.diagnostic
 * @param {(commandId: string, argumentsJson: string) => void} props.onFix
 * @param {string} [props.fixLabel]   the fix button's text when the catalog has no fix text
 */
export function DiagnosticView({ diagnostic, onFix, fixLabel = "Fix it" }) {
  const mode = useMode();
  const text = diagnosticText(diagnostic);
  const hasFix = diagnostic.fixCommandId !== "";
  return (
    <div
      className={`rr-diagnostic rr-diagnostic--${text.severity}`}
      role={text.severity === "error" ? "alert" : "status"}
    >
      <p className="rr-diagnostic__summary">
        <span className="rr-diagnostic__severity">{SEVERITY_LABELS[text.severity]}</span>{" "}
        <span className="rr-diagnostic__basic">{text.basicText}</span>
      </p>
      {mode === "advanced" ? (
        <AdvancedDetails diagnostic={diagnostic} title={text.title} fixText={text.fixText} />
      ) : null}
      {hasFix ? (
        <Button variant="primary" onClick={() => onFix(diagnostic.fixCommandId, diagnostic.fixArgumentsJson)}>
          {text.fixText || fixLabel}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * What Advanced adds after the Basic text: the code and title, where it happened, the full message and the fix.
 * @param {object} props
 * @param {Diagnostic} props.diagnostic
 * @param {string} props.title
 * @param {string} props.fixText
 */
function AdvancedDetails({ diagnostic, title, fixText }) {
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
      {fixText ? (
        <>
          <dt>Fix</dt>
          <dd>{fixText}</dd>
        </>
      ) : null}
    </dl>
  );
}
