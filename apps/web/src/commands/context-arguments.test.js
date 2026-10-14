// The context prefill: an argument takes a context value only when its name is a context name and the context holds a
// value for it.

import { describe, expect, test } from "vitest";

import { contextChips, flatContext, prefillArguments } from "./context-arguments.js";

describe("prefillArguments", () => {
  test("fills a context-named argument from the context, and leaves the rest", () => {
    const context = flatContext({ core: { run_id: "run_01" }, psychology: { sae_id: "sae_01" } });
    expect(prefillArguments(["run_id", "suite", "sae_id"], context)).toEqual({ run_id: "run_01" });
  });

  test("an empty context value fills nothing, and a later plugin's value wins", () => {
    expect(prefillArguments(["run_id"], flatContext({ core: { run_id: "" } }))).toEqual({});
    expect(flatContext({ a: { run_id: "run_1" }, b: { run_id: "run_2" } }).run_id).toBe("run_2");
  });

  test("the chips show each prefilled argument as text", () => {
    expect(contextChips({ run_id: "run_01" })).toEqual([{ name: "run_id", value: "run_01" }]);
  });
});
