// The first-party plugin UIs this build knows (layout frontend-and-plugins.md 8.1, 8.5). Trusted and in-process only:
// a static table, reviewed like code, so no third-party code ever loads into the app. It is empty until the
// first-party plugins land (Psychology and Evaluation, M4 and M5); each adds one line here and nothing else.

/**
 * A plugin id to a function that loads its UI and resolves to the PluginUi (contracts 5.5).
 * @type {Object<string, () => Promise<import("@rastai/plugin").PluginUi>>}
 */
export const FIRST_PARTY_UI_LOADERS = {};
