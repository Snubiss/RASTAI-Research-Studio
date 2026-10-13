// The one React context that carries the effective mode (layout frontend-and-plugins.md 2.4, decision 4). Private
// to mode/: callers use ModeProvider, useMode and AdvancedOnly.

import { createContext } from "react";

/** @typedef {import("./mode-types.js").UiMode} UiMode */

// ui.mode's prototype default is advanced (contracts 11.2); a component rendered outside any provider gets it too.
/** @type {UiMode} */
export const DEFAULT_MODE = "advanced";

export const ModeContext = createContext(/** @type {UiMode} */ (DEFAULT_MODE));
