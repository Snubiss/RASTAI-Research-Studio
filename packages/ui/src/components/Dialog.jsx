// A modal dialog on the browser's own <dialog> element: showModal() makes the rest of the page inert, traps focus
// and closes on Escape, so none of that is hand-written. Controlled by `open`; Escape and the close button both
// call onClose, and the caller decides.

import { useEffect, useId, useRef } from "react";

import { Button } from "./Button.jsx";

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {boolean} props.open
 * @param {string} props.title
 * @param {() => void} props.onClose
 * @param {ReactNode} props.children
 * @param {ReactNode} [props.actions]   the buttons at the bottom, usually a primary and a cancel Button
 * @param {string} [props.closeLabel]
 */
export function Dialog({ open, title, onClose, children, actions = null, closeLabel = "Close" }) {
  const dialogRef = useRef(/** @type {HTMLDialogElement | null} */ (null));
  const titleId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) {
      return;
    }
    if (open && !dialog.open) {
      dialog.showModal();
    }
    if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  /** @param {import("react").SyntheticEvent} event */
  function handleCancel(event) {
    // Escape: keep the dialog open until the caller says otherwise, so `open` stays the one source of truth.
    event.preventDefault();
    onClose();
  }

  return (
    <dialog ref={dialogRef} className="rr-dialog" aria-labelledby={titleId} onCancel={handleCancel}>
      <div className="rr-dialog__header">
        <h2 className="rr-dialog__title" id={titleId}>
          {title}
        </h2>
        <Button variant="quiet" onClick={onClose} ariaLabel={closeLabel}>
          {closeLabel}
        </Button>
      </div>
      <div className="rr-dialog__body">{children}</div>
      {actions ? <div className="rr-dialog__actions">{actions}</div> : null}
    </dialog>
  );
}
