// The one React context that carries a subtree's PluginHost (layout frontend-and-plugins.md 7.1). Private to
// react/: callers use PluginHostProvider and usePluginHost.

import { createContext } from "react";

/** @typedef {import("../host-api.js").PluginHost} PluginHost */

export const PluginHostContext = createContext(/** @type {PluginHost | null} */ (null));
