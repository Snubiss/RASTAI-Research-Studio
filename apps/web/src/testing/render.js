// Test support for the browser tests (`*.browser.test.jsx`): mounts a React element into the real page and removes it
// afterwards. Not part of the app. React's act() flushes effects and state updates before a test looks at the page.

import { act } from "react";
import { createRoot } from "react-dom/client";

/** @typedef {import("react").ReactNode} ReactNode */

// React warns unless the page says it runs inside act(), as a test runner does.
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

/**
 * @param {ReactNode} element
 * @returns {Promise<{ container: HTMLElement, unmount: () => Promise<void> }>}
 */
export async function mount(element) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(element));
  return {
    container,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * Runs something that changes state (a click, an event, an answer arriving) inside act().
 * @param {() => void | Promise<void>} action
 */
export async function inAct(action) {
  await act(async () => {
    await action();
  });
}
