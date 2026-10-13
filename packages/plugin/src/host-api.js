// The one host interface every section, panel and plugin UI codes against (layout frontend-and-plugins.md 7.2;
// contracts 12.4, owner decision N5). JSDoc only: nothing here runs. Three objects have the PluginHost shape: the
// in-process host (in-process-host.js), the mock host (testing/mock-host.js) and, from M8, the iframe guest.
// Every field is plain data or a plain function, so the same shape can cross the iframe bridge later: a host never
// hands out DOM nodes, the shell's window or a DaemonClient.

/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("@rastai/client").ExecuteFunction} ExecuteFunction */
/** @typedef {import("@rastai/client").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("@rastai/client").ExecuteOptions} ExecuteOptions */
/** @typedef {import("@rastai/ui").UiMode} UiMode */
/** @typedef {import("@rastai/ui").ThemeTokens} ThemeTokens */

/**
 * Which bus events a subscriber wants.
 * @typedef {object} EventFilter
 * @property {string[]} eventTypes   e.g. ["job.state_changed"]
 * @property {string[]} subjectIds   empty: any subject
 */

/**
 * One entry of an enabled plugin's manifest list, e.g. a lens or a metric.
 * @typedef {object} Contribution
 * @property {string} pluginId
 * @property {string} id
 * @property {string} title
 * @property {object} data            the manifest entry as it is
 */

/**
 * What every panel, section and plugin UI gets. It has `execute`, so it is a CommandExecutor (contracts 12.4) and
 * the generated wrappers accept it: `jobList(host, {})` works as `jobList(client, {})` does in a script.
 * @typedef {object} PluginHost
 * @property {string} pluginId
 * @property {ExecuteFunction} execute
 * @property {(filter: EventFilter, onEvent: (event: BusEvent) => void) => (() => void)} subscribe   returns unsubscribe
 * @property {() => UiMode} mode
 * @property {() => ThemeTokens} theme                 resolved tokens, locked ones excluded
 * @property {(listener: () => void) => (() => void)} onDisplayChange   mode or theme changed; returns unsubscribe
 * @property {(route: string) => void} navigate        e.g. "#/psychology/replay/conv_..."
 * @property {(values: Object<string, string>) => void} setContext   palette prefill and keybinding "when"
 * @property {(kind: string) => Contribution[]} contributions   kind is a manifest contributes key, e.g. "lenses"
 * @property {(diagnostics: Diagnostic[]) => void} showDiagnostics
 */

export {};
