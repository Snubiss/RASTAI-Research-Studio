// The signed-out screen (RRS-U0003): the session ended or a reload dropped the in-memory session key, so the
// window can do nothing until Rastai is opened again (layout frontend-and-plugins.md 2.10 and 8.3). It covers the
// whole window and is drawn only with locked colors, so a page can't fake a working app over it.

import { CATALOG } from "@rastai/catalog";

import { CODE_STYLE, HEADING_STYLE, PANEL_STYLE, SCREEN_STYLE, TEXT_STYLE } from "./locked-styles.js";

const SIGNED_OUT_CODE = "RRS-U0003";
// How to get a new sign-in link: the desktop app, or the CLI's `rastai open` (contracts 3.3).
const REOPEN_INSTRUCTIONS = "Open the Rastai app, or run `rastai open` in a terminal. It opens a new signed-in window.";

export function SignedOutScreen() {
  const entry = CATALOG.find((item) => item.code === SIGNED_OUT_CODE);
  return (
    <div className="rr-locked" style={SCREEN_STYLE} role="alert">
      <div style={PANEL_STYLE}>
        <h1 style={HEADING_STYLE}>{entry?.basic_text}</h1>
        <p style={TEXT_STYLE}>{entry?.fix_text}</p>
        <p style={TEXT_STYLE}>{REOPEN_INSTRUCTIONS}</p>
        <p style={TEXT_STYLE}>
          <code style={CODE_STYLE}>{SIGNED_OUT_CODE}</code>
        </p>
      </div>
    </div>
  );
}
