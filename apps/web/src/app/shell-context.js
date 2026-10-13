// Gives the frame's components the window's shell (app/shell.js). The App provides it once; components read it with
// useShell. A component outside the App has no shell and fails loudly, which is a programming error, not a user error.

import { createContext, useContext } from "react";

/** @typedef {import("./shell.js").Shell} Shell */

const ShellContext = createContext(/** @type {Shell | null} */ (null));

export const ShellProvider = ShellContext.Provider;

/**
 * @returns {Shell}
 */
export function useShell() {
  const shell = useContext(ShellContext);
  if (shell === null) {
    throw new Error("useShell needs the App's ShellProvider above the component.");
  }
  return shell;
}
