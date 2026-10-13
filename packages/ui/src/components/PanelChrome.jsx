// A panel's title bar: its title, the Basic/Advanced override for this panel and the panel menu (layout
// frontend-and-plugins.md 2.4). apps/web's PanelFrame renders it above the panel and turns the override into the
// panel's ModeProvider. The menu follows the WAI-ARIA menu-button pattern: the items are not Tab stops, the arrow,
// Home and End keys move among them, and Escape or Tab closes it.

import { useEffect, useId, useRef, useState } from "react";

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
  const triggerRef = useRef(/** @type {HTMLButtonElement | null} */ (null));
  const menuRef = useRef(/** @type {HTMLUListElement | null} */ (null));

  // The first item takes focus when the menu opens, so the keyboard is in the menu at once (WAI-ARIA menu button).
  useEffect(() => {
    if (menuOpen) {
      menuItemsOf(menuRef.current)[0]?.focus();
    }
  }, [menuOpen]);

  // Closing the menu returns focus to its button; otherwise focus falls to the page body.
  function closeMenu() {
    setMenuOpen(false);
    triggerRef.current?.focus();
  }

  /** @param {string} itemId */
  function selectItem(itemId) {
    closeMenu();
    onMenuSelect(itemId);
  }

  // Keys are handled only while the menu is open: a closed menu must not swallow an Escape meant for a dialog around it.
  /** @param {import("react").KeyboardEvent} event */
  function handleMenuKeyDown(event) {
    if (!menuOpen) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeMenu();
    } else if (event.key === "Tab") {
      // Focus returns to the menu button first, so the browser's own Tab step leaves the menu from there.
      closeMenu();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      moveMenuFocus(menuRef.current, event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const items = menuItemsOf(menuRef.current);
      items[event.key === "Home" ? 0 : items.length - 1]?.focus();
    }
  }

  // Focus that leaves the menu for something outside it (a click elsewhere) closes the menu too.
  /** @param {import("react").FocusEvent} event */
  function handleMenuBlur(event) {
    const next = event.relatedTarget;
    const leavingMenu = !(next instanceof Node) || !event.currentTarget.contains(next);
    if (menuOpen && leavingMenu) {
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
        <div className="rr-panel-chrome__menu" onKeyDown={handleMenuKeyDown} onBlur={handleMenuBlur}>
          <button
            ref={triggerRef}
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
            <ul ref={menuRef} className="rr-menu" id={menuId} role="menu" aria-label={menuLabel}>
              {menuItems.map((item) => (
                <li key={item.id} role="none">
                  <button
                    type="button"
                    role="menuitem"
                    className="rr-menu__item"
                    tabIndex={-1}
                    onClick={() => selectItem(item.id)}
                  >
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
 * The menu's items, in order. Only role=menuitem buttons count, so a separator added later is skipped.
 * @param {HTMLElement | null} menu
 * @returns {HTMLElement[]}
 */
function menuItemsOf(menu) {
  if (!menu) {
    return [];
  }
  return [...menu.querySelectorAll('[role="menuitem"]')].map((item) => /** @type {HTMLElement} */ (item));
}

/**
 * Moves focus to the next (step 1) or previous (step -1) item, wrapping at the ends. With no item focused yet, the
 * first item (going down) or the last (going up) takes it.
 * @param {HTMLElement | null} menu
 * @param {number} step
 */
function moveMenuFocus(menu, step) {
  const items = menuItemsOf(menu);
  if (items.length === 0) {
    return;
  }
  const current = items.indexOf(/** @type {HTMLElement} */ (document.activeElement));
  if (current < 0) {
    items[step > 0 ? 0 : items.length - 1].focus();
    return;
  }
  const next = (current + step + items.length) % items.length;
  items[next].focus();
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
