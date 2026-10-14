// The palette's subsequence filter (layout frontend-and-plugins.md 2.5): letters in order, ignoring case.

import { describe, expect, test } from "vitest";

import { isMatch, matchRank } from "./palette-match.js";

describe("the subsequence match", () => {
  test("a subsequence matches, ignoring case", () => {
    expect(isMatch(matchRank("trn", "Training"))).toBe(true);
    expect(isMatch(matchRank("AGT", "Agent Gym"))).toBe(true);
  });

  test("letters out of order don't match", () => {
    expect(isMatch(matchRank("nt", "Training"))).toBe(false);
  });

  test("an empty query matches everything, and surrounding spaces are ignored", () => {
    expect(isMatch(matchRank("", "Home"))).toBe(true);
    expect(isMatch(matchRank("  ", "Home"))).toBe(true);
    expect(isMatch(matchRank(" ho ", "Home"))).toBe(true);
  });
});

describe("matchRank", () => {
  test("a match at the start ranks above one inside the text, which ranks above a scattered one", () => {
    const prefix = matchRank("set", "Settings");
    const contains = matchRank("set", "Reset layout");
    const scattered = matchRank("set", "Stream every token");
    expect(prefix).toBeLessThan(contains);
    expect(contains).toBeLessThan(scattered);
    expect(scattered).toBeGreaterThanOrEqual(0);
  });

  test("no match is -1, and an empty query ranks every text the same", () => {
    expect(matchRank("xyz", "Training")).toBe(-1);
    expect(matchRank("", "Training")).toBe(matchRank("", "Home"));
  });
});
