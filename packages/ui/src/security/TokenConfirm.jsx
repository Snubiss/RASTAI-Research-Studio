// The "create token" confirmation (ui-design.md 5.2; contracts 6.3): what the new token can do, what it cannot, and a
// box to tick because the token is shown once. Approve is disabled until the box is ticked, so a stray Enter never
// creates a token nobody saw. Cancel is outlined and has the focus first. Drawn only with locked colors, so a theme
// can't make the list of what the token can do look smaller.

import { useEffect, useId, useRef, useState } from "react";

import {
  BUTTON_STYLE,
  CHECKBOX_LABEL_STYLE,
  CHECKBOX_STYLE,
  DISABLED_BUTTON_STYLE,
  HEADING_STYLE,
  LIST_STYLE,
  PANEL_STYLE,
  PRIMARY_BUTTON_STYLE,
  TEXT_STYLE,
} from "./locked-styles.js";
import { LockedFocusRing } from "./LockedFocusRing.jsx";

const SHOW_ONCE_TEXT = "I understand this token is shown once and cannot be shown again.";

/**
 * @param {object} props
 * @param {string} props.tokenName       the name the user gave the token
 * @param {string[]} props.canDo         what the token allows, in plain English
 * @param {string[]} props.cannotDo      what it never allows, in plain English
 * @param {() => void} props.onApprove
 * @param {() => void} props.onCancel
 */
export function TokenConfirm({ tokenName, canDo, cannotDo, onApprove, onCancel }) {
  const dialogRef = useRef(/** @type {HTMLDialogElement | null} */ (null));
  const cancelRef = useRef(/** @type {HTMLButtonElement | null} */ (null));
  const titleId = useId();
  const [acknowledged, setAcknowledged] = useState(false);

  // The dialog's own focus rule would pick the checkbox (the first control in the page order), so Cancel is focused
  // here, after the dialog opens; autoFocus alone runs before the dialog is open and the focus is lost.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) {
      dialog.showModal();
    }
    cancelRef.current?.focus();
    return () => dialog?.close();
  }, []);

  /** @param {import("react").SyntheticEvent} event */
  function handleCancel(event) {
    event.preventDefault();
    onCancel();
  }

  function approve() {
    if (acknowledged) {
      onApprove();
    }
  }

  return (
    <dialog ref={dialogRef} className="rr-locked" style={PANEL_STYLE} aria-labelledby={titleId} onCancel={handleCancel}>
      <LockedFocusRing />
      <h2 id={titleId} style={HEADING_STYLE}>
        Create token {tokenName}?
      </h2>
      <p style={TEXT_STYLE}>This token can:</p>
      <ul style={LIST_STYLE}>
        {canDo.map((item) => (
          <li key={item} style={TEXT_STYLE}>
            {item}
          </li>
        ))}
      </ul>
      <p style={TEXT_STYLE}>This token cannot:</p>
      <ul style={LIST_STYLE}>
        {cannotDo.map((item) => (
          <li key={item} style={TEXT_STYLE}>
            {item}
          </li>
        ))}
      </ul>
      <label style={CHECKBOX_LABEL_STYLE}>
        <input
          type="checkbox"
          className="rr-locked"
          style={CHECKBOX_STYLE}
          checked={acknowledged}
          onChange={(event) => setAcknowledged(event.target.checked)}
        />
        <span>{SHOW_ONCE_TEXT}</span>
      </label>
      <div>
        <button ref={cancelRef} type="button" className="rr-locked" style={BUTTON_STYLE} onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="rr-locked"
          style={acknowledged ? PRIMARY_BUTTON_STYLE : DISABLED_BUTTON_STYLE}
          disabled={!acknowledged}
          onClick={approve}
        >
          Create token
        </button>
      </div>
    </dialog>
  );
}
