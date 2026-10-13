// The effective mode, for components that change their wording or detail (layout frontend-and-plugins.md 2.4).

import { useContext } from "react";

import { ModeContext } from "./mode-context.js";

/** @typedef {import("./mode-types.js").UiMode} UiMode */

/** @returns {UiMode} */
export function useMode() {
  return useContext(ModeContext);
}
