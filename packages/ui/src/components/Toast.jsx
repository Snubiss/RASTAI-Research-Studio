// The toast area: short messages in a polite live region, so screen readers read them without moving focus. The
// list comes from the caller (showToast and dismissToast in show-toast.js).

import { Button } from "./Button.jsx";

/** @typedef {import("./show-toast.js").ToastEntry} ToastEntry */

/**
 * @param {object} props
 * @param {ToastEntry[]} props.toasts
 * @param {(toastId: string) => void} props.onDismiss
 * @param {string} [props.dismissLabel]
 */
export function Toast({ toasts, onDismiss, dismissLabel = "Dismiss" }) {
  return (
    <div className="rr-toasts" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`rr-toast rr-toast--${toast.tone}`}>
          <p className="rr-toast__message">{toast.message}</p>
          <Button variant="quiet" onClick={() => onDismiss(toast.id)} ariaLabel={`${dismissLabel}: ${toast.message}`}>
            {dismissLabel}
          </Button>
        </div>
      ))}
    </div>
  );
}
