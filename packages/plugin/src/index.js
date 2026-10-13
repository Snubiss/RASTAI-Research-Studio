// @rastai/plugin: the only import path sections and plugin UIs use (layout frontend-and-plugins.md 7). Public
// exports only. The typed command wrappers come from "@rastai/client/commands", which imports nothing at run time,
// so they bring no transport code into a plugin bundle. Nothing of @rastai/ui's security UI is or may be exported
// here (layout 2.10): plugin UIs can't render the security UI, so they can't fake it.

// One plain function per core command, each taking any CommandExecutor: modelLoad(host, args) in a panel is the
// same call as modelLoad(client, args) in a script (contracts 12.4).
export * from "@rastai/client/commands";

export { createInProcessHost, CORE_PLUGIN_ID } from "./in-process-host.js";
export { registerPluginUi, registeredPluginUi } from "./ui-registry.js";

export { createStore, useStore } from "./state/store.js";
export { createLiveBuffer, pushItem, itemsSince, useLiveBuffer } from "./state/live-buffer.js";
export { loadDraft, saveDraft, clearDraft } from "./state/draft-store.js";
export { invalidateQueries } from "./state/query-cache.js";

export { PluginHostProvider } from "./react/PluginHostProvider.jsx";
export { usePluginHost } from "./react/use-plugin-host.js";
export { useQuery } from "./react/use-query.js";
export { useCommand } from "./react/use-command.js";
export { CommandForm } from "./react/CommandForm.jsx";
export { JobProgress } from "./react/JobProgress.jsx";

export { createMockHost } from "./testing/mock-host.js";

/** @typedef {import("./host-api.js").PluginHost} PluginHost */
/** @typedef {import("./host-api.js").EventFilter} EventFilter */
/** @typedef {import("./host-api.js").Contribution} Contribution */
/** @typedef {import("./in-process-host.js").HostServices} HostServices */
/** @typedef {import("./ui-registry.js").PluginUi} PluginUi */
/** @typedef {import("./ui-registry.js").Section} Section */
/** @typedef {import("./ui-registry.js").Panel} Panel */
/** @typedef {import("./ui-registry.js").LensView} LensView */
/** @typedef {import("./ui-registry.js").MetricView} MetricView */
/** @typedef {import("./ui-registry.js").SettingsPage} SettingsPage */
/** @typedef {import("./state/store.js").Store} Store */
/** @typedef {import("./state/live-buffer.js").LiveBuffer} LiveBuffer */
/** @typedef {import("./state/query-cache.js").QueryState} QueryState */
/** @typedef {import("./react/use-command.js").CommandState} CommandState */
/** @typedef {import("./testing/mock-host.js").MockHost} MockHost */
/** @typedef {import("./testing/mock-host.js").RecordedCall} RecordedCall */
