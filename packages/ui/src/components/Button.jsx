// A push button. Every button in the app is this one, so they share one look, one focus ring and one target size.

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {ReactNode} props.children
 * @param {() => void} [props.onClick]
 * @param {"primary" | "secondary" | "quiet"} [props.variant]   primary is the accent fill; quiet has no border
 * @param {boolean} [props.disabled]
 * @param {"button" | "submit"} [props.type]
 * @param {string} [props.title]
 * @param {string} [props.ariaLabel]                              for a button whose text alone doesn't say what it does
 */
export function Button({
  children,
  onClick,
  variant = "secondary",
  disabled = false,
  type = "button",
  title,
  ariaLabel,
}) {
  return (
    <button
      type={type}
      className={`rr-button rr-button--${variant}`}
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={ariaLabel}
    >
      {children}
    </button>
  );
}
