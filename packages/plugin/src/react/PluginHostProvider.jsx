// Gives a subtree its PluginHost: the app's PanelFrame renders one around each panel with that plugin's host, and
// a test renders one with a mock host (layout frontend-and-plugins.md 7.1). Everything below reaches the daemon
// only through this host.

import { PluginHostContext } from "./plugin-host-context.js";

/** @typedef {import("../host-api.js").PluginHost} PluginHost */
/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @param {object} props
 * @param {PluginHost} props.host
 * @param {ReactNode} props.children
 */
export function PluginHostProvider({ host, children }) {
  return <PluginHostContext.Provider value={host}>{children}</PluginHostContext.Provider>;
}
