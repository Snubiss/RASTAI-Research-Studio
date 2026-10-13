// The effective settings as the daemon answers settings.get (contracts 11.1, 11.2). The app reads them, and the
// window refetches them on every boot and after command.finished for settings.set (layout 8.1, 2.3). Nothing here
// stores a setting: the daemon's settings.json files do.

/**
 * The value of one dotted key in the merged settings, or the fallback when the daemon didn't send it.
 * @param {any} values   the settings.get result's `values`: the merged sections, as in a settings.json file
 * @param {string} dottedKey   e.g. "ui.theme"
 * @param {any} fallback
 * @returns {any}
 */
export function readSetting(values, dottedKey, fallback) {
  const [section, name] = dottedKey.split(".");
  const value = values?.[section]?.[name];
  return value === undefined || value === null ? fallback : value;
}

/**
 * The settings.get result a store holds before any answer: no values, so every read falls back to its default.
 * @returns {{ values: any, sources: any[] }}
 */
export function emptySettings() {
  return { values: {}, sources: [] };
}
