// The top bar (design 3.2): the logo (it goes Home), the project (none in this version), the breadcrumb of the current
// route, the palette entry, and the global Basic/Advanced switch. The switch is ui.mode through settings.set.

import { SegmentedControl } from "@rastai/ui";
import { useStore } from "@rastai/plugin";

import { useShell } from "../app/shell-context.js";
import { HOME_SECTION_ID, SECTION_LABELS } from "../constants.js";
import { sectionFragment } from "../routing/route.js";
import { sectionLabel } from "../state/mode.js";

const MODE_OPTIONS = [
  { value: "basic", label: "Basic" },
  { value: "advanced", label: "Advanced" },
];

/**
 * @param {object} props
 * @param {() => void} props.onOpenPalette
 */
export function TopBar({ onOpenPalette }) {
  const shell = useShell();
  const display = useStore(shell.stores.display);
  const route = useStore(shell.stores.route);
  const sectionName = sectionLabel(route.sectionId, display.mode);
  return (
    <header className="rr-top-bar">
      <a className="rr-top-bar__logo" href={sectionFragment(HOME_SECTION_ID)}>
        Rastai
      </a>
      <span className="rr-top-bar__project" aria-label="Project">
        No project
      </span>
      <nav aria-label="Breadcrumb" className="rr-top-bar__crumbs">
        <ol>
          <li aria-current="page">{sectionName}</li>
        </ol>
      </nav>
      <button
        type="button"
        className="rr-top-bar__palette"
        onClick={onOpenPalette}
        aria-keyshortcuts="Control+K Meta+K"
      >
        <span>Search or run a command</span>
        <kbd>Ctrl K</kbd>
      </button>
      <SegmentedControl
        label="Mode"
        options={MODE_OPTIONS}
        value={display.mode}
        onChange={(value) => shell.setMode(/** @type {"basic" | "advanced"} */ (value))}
      />
    </header>
  );
}
