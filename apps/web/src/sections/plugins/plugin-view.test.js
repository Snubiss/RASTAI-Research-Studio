// The Plugins list's words: the trust line counts the permissions still waiting, and the tone follows the state.

import { describe, expect, test } from "vitest";

import { stateTone, trustLine } from "./plugin-view.js";

describe("trustLine", () => {
  test("says when no permission is asked for, when all are granted, and how many wait", () => {
    expect(trustLine({ manifest: { permissions: [] }, granted_permissions: [] })).toBe("Asks for no permissions");
    expect(trustLine({ manifest: { permissions: ["read", "gpu"] }, granted_permissions: ["read", "gpu"] })).toBe(
      "Every permission it asks for is granted",
    );
    expect(trustLine({ manifest: { permissions: ["read", "gpu"] }, granted_permissions: ["read"] })).toBe(
      "1 of 2 permissions wait for your review",
    );
  });
});

describe("stateTone", () => {
  test("enabled is ok, waiting for review is a warning, incompatible is an error, anything else neutral", () => {
    expect(stateTone("enabled")).toBe("ok");
    expect(stateTone("awaiting_review")).toBe("warning");
    expect(stateTone("incompatible")).toBe("error");
    expect(stateTone("disabled")).toBe("neutral");
  });
});
