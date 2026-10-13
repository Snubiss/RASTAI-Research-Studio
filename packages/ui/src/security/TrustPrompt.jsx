// The trust prompt: what a plugin asks for, in plain English, before it is installed or gains a permission
// (owner decision N7; contracts 6.3 and 9.7; layout frontend-and-plugins.md 2.10). The shell renders it as a modal
// above every panel, so no plugin UI can cover it. It always says that a plugin's Python code runs as trusted code.
// It carries the locked "Rastai security check" header (ui-design.md 5.2). Escape declines, and "Don't install" has
// the focus first, so a stray Enter never installs.

import { useEffect, useId, useRef } from "react";

import { BUTTON_STYLE, DANGER_TEXT_STYLE, HEADING_STYLE, PANEL_STYLE, TEXT_STYLE } from "./locked-styles.js";
import { LockedFocusRing } from "./LockedFocusRing.jsx";
import { PermissionList } from "./PermissionList.jsx";
import { SecurityCheckHeader } from "./SecurityCheckHeader.jsx";

const TRUSTED_CODE_WARNING =
  "A plugin can include Python code. That code runs on this computer as trusted code, with your access to files " +
  "and programs. Install only plugins you trust.";

/**
 * @param {object} props
 * @param {string} props.pluginName
 * @param {string[]} props.permissions   the manifest's permissions[]
 * @param {() => void} props.onAccept
 * @param {() => void} props.onDecline
 */
export function TrustPrompt({ pluginName, permissions, onAccept, onDecline }) {
  const dialogRef = useRef(/** @type {HTMLDialogElement | null} */ (null));
  const titleId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) {
      dialog.showModal();
    }
    return () => dialog?.close();
  }, []);

  /** @param {import("react").SyntheticEvent} event */
  function handleCancel(event) {
    event.preventDefault();
    onDecline();
  }

  return (
    <dialog ref={dialogRef} className="rr-locked" style={PANEL_STYLE} aria-labelledby={titleId} onCancel={handleCancel}>
      <LockedFocusRing />
      <SecurityCheckHeader />
      <h2 id={titleId} style={HEADING_STYLE}>
        Install {pluginName}?
      </h2>
      <p style={TEXT_STYLE}>It asks for these permissions:</p>
      <PermissionList requested={permissions} />
      <p style={DANGER_TEXT_STYLE}>{TRUSTED_CODE_WARNING}</p>
      <div>
        <button type="button" className="rr-locked" style={BUTTON_STYLE} onClick={onDecline} autoFocus>
          {"Don't install"}
        </button>
        <button type="button" className="rr-locked" style={BUTTON_STYLE} onClick={onAccept}>
          Install
        </button>
      </div>
    </dialog>
  );
}
