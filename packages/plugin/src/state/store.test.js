// createStore (layout frontend-and-plugins.md 2.3): a plain get, set and subscribe. useStore is checked in a real
// browser in react/hooks.browser.test.jsx.

import { describe, expect, test } from "vitest";

import { createStore } from "./store.js";

describe("createStore", () => {
  test("get returns the latest value and set tells every listener", () => {
    const store = createStore("#/home");
    /** @type {string[]} */
    const heard = [];
    store.subscribe(() => heard.push(`first ${store.get()}`));
    store.subscribe(() => heard.push(`second ${store.get()}`));
    store.set("#/inference");
    expect(store.get()).toBe("#/inference");
    expect(heard).toEqual(["first #/inference", "second #/inference"]);
  });

  test("an unsubscribed listener hears nothing more", () => {
    const store = createStore(0);
    let calls = 0;
    const unsubscribe = store.subscribe(() => {
      calls += 1;
    });
    store.set(1);
    unsubscribe();
    store.set(2);
    expect(calls).toBe(1);
  });
});
