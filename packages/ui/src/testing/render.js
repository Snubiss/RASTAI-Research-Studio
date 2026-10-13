// Test support for the browser tests (`*.browser.test.jsx`): mount a React element into the real page and remove
// it afterwards. Not part of the package's API. React's act() flushes effects and state updates before the test
// looks at the page.

import { act } from "react";
import { createRoot } from "react-dom/client";

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @typedef {object} Mounted
 * @property {HTMLElement} container
 * @property {(element: ReactNode) => Promise<void>} rerender
 * @property {() => Promise<void>} unmount
 */

// React warns unless the page says it runs inside act(), as a test runner does.
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

/**
 * @param {ReactNode} element
 * @returns {Promise<Mounted>}
 */
export async function mount(element) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(element));
  return {
    container,
    rerender: async (next) => {
      await act(async () => root.render(next));
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * Runs a user action (a click, a key) inside act(), so React has re-rendered before the test checks the page.
 * @param {() => void | Promise<void>} action
 */
export async function inAct(action) {
  await act(async () => {
    await action();
  });
}
