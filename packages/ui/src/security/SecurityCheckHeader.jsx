// The "Rastai security check" line above the title of every locked prompt (ui-design.md 5.2). The lock glyph is drawn
// as a small SVG, not a character, so no font can swap it for another symbol. It is hidden from screen readers
// because the words say the same thing.

import { HEADER_STYLE, LOCK_GLYPH_STYLE } from "./locked-styles.js";

const HEADER_TEXT = "Rastai security check";

export function SecurityCheckHeader() {
  return (
    <p style={HEADER_STYLE}>
      <svg
        aria-hidden="true"
        focusable="false"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        style={LOCK_GLYPH_STYLE}
      >
        <rect x="4" y="11" width="16" height="10" rx="2" />
        <path d="M8 11V7a4 4 0 0 1 8 0v4" />
      </svg>
      <span>{HEADER_TEXT}</span>
    </p>
  );
}
