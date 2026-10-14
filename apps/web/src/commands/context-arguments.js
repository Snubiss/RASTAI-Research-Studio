// Prefilled arguments (layout frontend-and-plugins.md 2.5 step 6): an argument named like a context id takes the value
// the current context holds, so "run_id" needs no typing when a run is open. Sections set the context with setContext;
// this file only reads it. Pure, so it is tested in Node.

// The argument names the context can fill. The names are the layout's list; a command argument with any other name is
// never prefilled.
export const CONTEXT_ARGUMENT_NAMES = ["conversation_id", "document_id", "run_id", "checkpoint_id", "model_ref"];

/**
 * The context values of every plugin in one object. A later plugin's value wins for the same key, as in the store's
 * own order.
 * @param {Object<string, Object<string, any>>} contextByPlugin   the context store: { pluginId: { key: value } }
 * @returns {Object<string, any>}
 */
export function flatContext(contextByPlugin) {
  /** @type {Object<string, any>} */
  const values = {};
  for (const pluginValues of Object.values(contextByPlugin)) {
    Object.assign(values, pluginValues);
  }
  return values;
}

/**
 * The argument values the context fills for one command: only the names the command takes and the context holds.
 * @param {string[]} argumentNames   the command's argument names
 * @param {Object<string, any>} context   flatContext's result
 * @returns {Object<string, any>}
 */
export function prefillArguments(argumentNames, context) {
  /** @type {Object<string, any>} */
  const prefilled = {};
  for (const name of argumentNames) {
    if (CONTEXT_ARGUMENT_NAMES.includes(name) && context[name] !== undefined && context[name] !== "") {
      prefilled[name] = context[name];
    }
  }
  return prefilled;
}

/**
 * The chips the palette shows under its input: each prefilled argument as "name = value".
 * @param {Object<string, any>} prefilled
 * @returns {{ name: string, value: string }[]}
 */
export function contextChips(prefilled) {
  return Object.entries(prefilled).map(([name, value]) => ({ name, value: String(value) }));
}
