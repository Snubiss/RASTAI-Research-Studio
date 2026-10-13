// Unsent drafts (layout frontend-and-plugins.md 2.3 and 7.5): the save is debounced, old drafts are dropped on load,
// and a localStorage that throws (a sandboxed iframe) makes the store a quiet no-op.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { clearDraft, loadDraft, saveDraft } from "./draft-store.js";

const KEY = "draft:inference:new";
const SAVE_DELAY_MS = 500;
const DAY_MS = 24 * 60 * 60 * 1000;

/** A localStorage stand-in over a Map; Node has none by default. */
function memoryStorage() {
  /** @type {Map<string, string>} */
  const values = new Map();
  return {
    values,
    /** @param {string} key */
    getItem: (key) => values.get(key) ?? null,
    /** @param {string} key @param {string} value */
    setItem: (key, value) => {
      values.set(key, value);
    },
    /** @param {string} key */
    removeItem: (key) => {
      values.delete(key);
    },
  };
}

function throwingStorage() {
  const refuse = () => {
    throw new Error("SecurityError: storage is disabled in this frame");
  };
  return { getItem: refuse, setItem: refuse, removeItem: refuse };
}

/** @type {ReturnType<typeof memoryStorage>} */
let storage;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-13T12:00:00Z"));
  storage = memoryStorage();
  vi.stubGlobal("localStorage", storage);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("saveDraft", () => {
  test("writes once per typing pause, with the last text", () => {
    saveDraft(KEY, "Hel");
    vi.advanceTimersByTime(SAVE_DELAY_MS - 1);
    saveDraft(KEY, "Hello");
    vi.advanceTimersByTime(SAVE_DELAY_MS - 1);
    expect(storage.values.has(KEY)).toBe(false);
    vi.advanceTimersByTime(1);
    expect(loadDraft(KEY)).toBe("Hello");
  });

  test("empty text clears the draft", () => {
    saveDraft(KEY, "Hello");
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    saveDraft(KEY, "");
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    expect(loadDraft(KEY)).toBeNull();
    expect(storage.values.has(KEY)).toBe(false);
  });
});

describe("loadDraft", () => {
  test("drops a draft older than 30 days", () => {
    saveDraft(KEY, "an old prompt");
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    vi.setSystemTime(Date.now() + 30 * DAY_MS + 1);
    expect(loadDraft(KEY)).toBeNull();
    expect(storage.values.has(KEY)).toBe(false);
  });

  test("keeps a draft just under 30 days old", () => {
    saveDraft(KEY, "a recent prompt");
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    vi.setSystemTime(Date.now() + 30 * DAY_MS - 1);
    expect(loadDraft(KEY)).toBe("a recent prompt");
  });

  test("ignores a stored value that isn't a draft", () => {
    storage.setItem(KEY, "not json");
    expect(loadDraft(KEY)).toBeNull();
    storage.setItem(KEY, '{"text":42}');
    expect(loadDraft(KEY)).toBeNull();
  });
});

describe("clearDraft", () => {
  test("removes the draft and cancels a save still waiting", () => {
    saveDraft(KEY, "sent already");
    clearDraft(KEY);
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    expect(loadDraft(KEY)).toBeNull();
  });
});

describe("with a localStorage that throws", () => {
  test("every call is a no-op", () => {
    vi.stubGlobal("localStorage", throwingStorage());
    expect(() => {
      saveDraft(KEY, "Hello");
      vi.advanceTimersByTime(SAVE_DELAY_MS);
      clearDraft(KEY);
    }).not.toThrow();
    expect(loadDraft(KEY)).toBeNull();
  });
});
