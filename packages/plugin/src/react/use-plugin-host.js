// The PluginHost of the panel or section a component is in (layout frontend-and-plugins.md 7.2).

import { useContext } from "react";

import { PluginHostContext } from "./plugin-host-context.js";

/** @typedef {import("../host-api.js").PluginHost} PluginHost */

/**
 * @returns {PluginHost}
 */
export function usePluginHost() {
  const host = useContext(PluginHostContext);
  if (host === null) {
    // A programming error, not a user-facing one: every panel the app or a test renders has a provider.
    throw new Error("usePluginHost needs a PluginHostProvider above the component.");
  }
  return host;
}
