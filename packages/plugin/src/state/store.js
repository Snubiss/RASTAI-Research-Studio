// Shared UI state: a value several components read, such as the route, the settings or a replay's playhead
// (layout frontend-and-plugins.md 2.3). A store is a plain object with get, set and subscribe; components read it
// with useStore, which is React's useSyncExternalStore. No state library.

import { useSyncExternalStore } from "react";

/**
 * A value that several components share. Plain object, no class.
 * @typedef {object} Store
 * @property {() => any} get
 * @property {(value: any) => void} set
 * @property {(listener: () => void) => (() => void)} subscribe   returns the unsubscribe function
 */

/**
 * @param {any} initialValue
 * @returns {Store}
 */
export function createStore(initialValue) {
  let value = initialValue;
  /** @type {Set<() => void>} */
  const listeners = new Set();
  return {
    get: () => value,
    set: (newValue) => {
      value = newValue;
      listeners.forEach((listener) => listener());
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/**
 * @param {Store} store
 * @returns {any} the current value; the component re-renders when it changes
 */
export function useStore(store) {
  return useSyncExternalStore(store.subscribe, store.get);
}
