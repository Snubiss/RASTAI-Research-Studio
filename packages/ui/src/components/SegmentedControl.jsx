// Two to four mutually exclusive options (ui-design.md 5.2): Basic or Advanced, a runtime toggle, a log filter. Each
// option is a button with aria-pressed, so keyboard users Tab to it and press Space or Enter, as for any button.

/**
 * @typedef {object} SegmentOption
 * @property {string} value
 * @property {string} label
 */

/**
 * @param {object} props
 * @param {string} props.label                   the group's name for screen readers, such as "Log stream"
 * @param {SegmentOption[]} props.options
 * @param {string} props.value                   the value of the pressed option
 * @param {(value: string) => void} props.onChange
 */
export function SegmentedControl({ label, options, value, onChange }) {
  return (
    <div className="rr-segmented" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className="rr-segmented__option"
          aria-pressed={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
