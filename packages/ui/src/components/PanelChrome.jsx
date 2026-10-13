// A panel's title bar: its title, the Basic/Advanced override for this panel and the panel menu (layout
// frontend-and-plugins.md 2.4). apps/web's PanelFrame renders it above the panel and turns the override into the
// panel's ModeProvider.

import { useId, useState } from "react";

import { Button } from "./Button.jsx";

/**
 * A panel's own mode: "" follows the app's ui.mode (the layout's empty mode_override).
 * @typedef {"" | "basic" | "advanced"} ModeOverride
 */

/**
 * One panel menu action ("Move to right", "Reload panel").
 * @typedef {object} PanelMenuItem
 * @property {string} id
 * @property {string} label
 */

const MODE_CHOICES = [
  { value: "", label: "Follow app" },
  { value: "basic", label: "Basic" },
  { value: "advanced", label: "Advanced" },
];

/**
 * @param {object} props
 * @param {string} props.title
 * @param {ModeOverride} props.modeOverride
 * @param {(modeOverride: ModeOverride) => void} props.onModeOverrideChange
 * @param {PanelMenuItem[]} [props.menuItems]
 * @param {(itemId: string) => void} [props.onMenuSelect]
 * @param {string} [props.menuLabel]
 */
export function PanelChrome({
  title,
  modeOverride,
  onModeOverrideChange,
  menuItems = [],
  onMenuSelect = () => {},
  menuLabel = "Panel menu",
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuId = useId();

  /** @param {string} itemId */
  function selectItem(itemId) {
    setMenuOpen(false);
    onMenuSelect(itemId);
  }

  /** @param {import("react").KeyboardEvent} event */
  function handleMenuKeyDown(event) {
    if (event.key === "Escape") {
      setMenuOpen(false);
    }
  }

  return (
    <div className="rr-panel-chrome">
      <h2 className="rr-panel-chrome__title">{title}</h2>
      <select
        className="rr-input rr-select rr-panel-chrome__mode"
        aria-label={`${title}: Basic or Advanced`}
        value={modeOverride}
        onChange={(event) => onModeOverrideChange(toModeOverride(event.target.value))}
      >
        {MODE_CHOICES.map((choice) => (
          <option key={choice.value} value={choice.value}>
            {choice.label}
          </option>
        ))}
      </select>
      {menuItems.length > 0 ? (
        <div className="rr-panel-chrome__menu" onKeyDown={handleMenuKeyDown}>
          <button
            type="button"
            className="rr-button rr-button--quiet"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            aria-controls={menuId}
            onClick={() => setMenuOpen(!menuOpen)}
          >
            {menuLabel}
          </button>
          {menuOpen ? (
            <ul className="rr-menu" id={menuId} role="menu" aria-label={menuLabel}>
              {menuItems.map((item) => (
                <li key={item.id} role="none">
                  <button type="button" role="menuitem" className="rr-menu__item" onClick={() => selectItem(item.id)}>
                    {item.label}
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * @param {string} value
 * @returns {ModeOverride}
 */
function toModeOverride(value) {
  if (value === "basic" || value === "advanced") {
    return value;
  }
  return "";
}
