// The "create token" prompt (ui-design.md 6.9 and 5.2; contracts 6.3). It shows the locked "Rastai security check"
// header, the token's name, "It can" and "It cannot" lists, and Cancel and Create token. The settings page makes the
// token when Create token is approved, then passes the new value back as tokenValue: the prompt then shows that value
// once in a box, with Copy. The prompt never makes a token itself. Cancel is outlined and takes the focus first, so a
// stray Enter declines. Create token is disabled as soon as it is pressed, so a double click can't approve twice; the
// caller closes the prompt if creating the token fails. Drawn only with locked colors, so a theme can't shrink the
// lists or hide the value.

import { useEffect, useId, useRef, useState } from "react";

import {
  BUTTON_STYLE,
  DISABLED_BUTTON_STYLE,
  HEADING_STYLE,
  LABEL_STYLE,
  LIST_STYLE,
  PANEL_STYLE,
  PRIMARY_BUTTON_STYLE,
  SHOW_ONCE_BOX_STYLE,
  TEXT_STYLE,
  TOKEN_VALUE_STYLE,
} from "./locked-styles.js";
import { LockedFocusRing } from "./LockedFocusRing.jsx";
import { SecurityCheckHeader } from "./SecurityCheckHeader.jsx";

const SHOW_ONCE_TEXT = "This token is shown once. Copy it now and store it somewhere safe; it cannot be shown again.";
const COPIED_TEXT = "Copied.";
const COPY_FAILED_TEXT = "Copy did not work. Select the token and copy it by hand.";

/**
 * @param {object} props
 * @param {string} props.tokenName                 the name the user gave the token
 * @param {string | null} [props.tokenValue]       the new token's value, set once Create token is approved
 * @param {string[]} props.canDo                   what the token allows, in plain English
 * @param {string[]} props.cannotDo                what it never allows, in plain English
 * @param {() => void} props.onApprove             Create token was pressed; the button stays disabled until the prompt closes
 * @param {() => void} props.onCancel              closes the prompt, before or after approval (Escape, Cancel, Done). The
 *                                                 caller cannot tell a close after approval from a cancel, so once tokenValue
 *                                                 has arrived it must already treat the token as created
 */
export function TokenConfirm({ tokenName, tokenValue = null, canDo, cannotDo, onApprove, onCancel }) {
  const dialogRef = useRef(/** @type {HTMLDialogElement | null} */ (null));
  const cancelRef = useRef(/** @type {HTMLButtonElement | null} */ (null));
  const copyRef = useRef(/** @type {HTMLButtonElement | null} */ (null));
  const titleId = useId();
  const [copyStatus, setCopyStatus] = useState("");
  const [approving, setApproving] = useState(false);

  // The dialog's own focus rule would pick the first control in the page order, so Cancel is focused here, after the
  // dialog opens; autoFocus alone runs before the dialog is open and the focus is lost.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) {
      dialog.showModal();
    }
    cancelRef.current?.focus();
    return () => dialog?.close();
  }, []);

  // Once the token exists, the focus moves to Copy: Create token has just left the page.
  useEffect(() => {
    if (tokenValue) {
      copyRef.current?.focus();
    }
  }, [tokenValue]);

  /** @param {import("react").SyntheticEvent} event */
  function handleCancel(event) {
    event.preventDefault();
    onCancel();
  }

  // A disabled button drops the focus, so the focus moves to Cancel first: the keyboard user keeps their place while
  // the token is made (ui-design.md 5.1, loading: aria-busy and a verb).
  function approve() {
    cancelRef.current?.focus();
    setApproving(true);
    onApprove();
  }

  async function copyToken() {
    try {
      await navigator.clipboard.writeText(tokenValue ?? "");
      setCopyStatus(COPIED_TEXT);
    } catch {
      setCopyStatus(COPY_FAILED_TEXT);
    }
  }

  return (
    <dialog
      ref={dialogRef}
      className="rr-locked"
      style={PANEL_STYLE}
      aria-labelledby={titleId}
      aria-busy={approving}
      onCancel={handleCancel}
    >
      <LockedFocusRing />
      <SecurityCheckHeader />
      <h2 id={titleId} style={HEADING_STYLE}>
        {`Create the token "${tokenName}"?`}
      </h2>
      <p style={LABEL_STYLE}>It can</p>
      <ul style={LIST_STYLE}>
        {canDo.map((item) => (
          <li key={item} style={TEXT_STYLE}>
            {item}
          </li>
        ))}
      </ul>
      <p style={LABEL_STYLE}>It cannot</p>
      <ul style={LIST_STYLE}>
        {cannotDo.map((item) => (
          <li key={item} style={TEXT_STYLE}>
            {item}
          </li>
        ))}
      </ul>
      {tokenValue ? (
        <div style={SHOW_ONCE_BOX_STYLE}>
          <p style={TEXT_STYLE}>{SHOW_ONCE_TEXT}</p>
          <code style={TOKEN_VALUE_STYLE}>{tokenValue}</code>
          <button ref={copyRef} type="button" className="rr-locked" style={BUTTON_STYLE} onClick={copyToken}>
            Copy
          </button>
          <p role="status" style={TEXT_STYLE}>
            {copyStatus}
          </p>
        </div>
      ) : null}
      <div>
        {tokenValue ? (
          <button type="button" className="rr-locked" style={PRIMARY_BUTTON_STYLE} onClick={onCancel}>
            Done
          </button>
        ) : (
          <>
            <button ref={cancelRef} type="button" className="rr-locked" style={BUTTON_STYLE} onClick={onCancel}>
              Cancel
            </button>
            <button
              type="button"
              className="rr-locked"
              style={approving ? DISABLED_BUTTON_STYLE : PRIMARY_BUTTON_STYLE}
              disabled={approving}
              onClick={approve}
            >
              {approving ? "Creating…" : "Create token"}
            </button>
          </>
        )}
      </div>
    </dialog>
  );
}
