// The palette's subsequence filter (layout frontend-and-plugins.md 2.5): letters in order, ignoring case.

import { describe, expect, test } from "vitest";

import { matchesQuery } from "./palette-match.js";

describe("matchesQuery", () => {
  test("a subsequence matches, ignoring case", () => {
    expect(matchesQuery("trn", "Training")).toBe(true);
    expect(matchesQuery("AGT", "Agent Gym")).toBe(true);
  });

  test("letters out of order don't match", () => {
    expect(matchesQuery("nt", "Training")).toBe(false);
  });

  test("an empty query matches everything, and surrounding spaces are ignored", () => {
    expect(matchesQuery("", "Home")).toBe(true);
    expect(matchesQuery("  ", "Home")).toBe(true);
    expect(matchesQuery(" ho ", "Home")).toBe(true);
  });
});
