// Sign-in (layout frontend-and-plugins.md 8.3): a window with no login code in its fragment has no session, and asks
// no one anything. The code exchange itself is @rastai/client's, tested there.

import { describe, expect, test } from "vitest";

import { signIn } from "./boot.js";
import { versionProblem } from "./daemon.js";

describe("signIn", () => {
  test("without a login code in the fragment there is no session key and no request", async () => {
    const result = await signIn(/** @type {any} */ ({ hash: "#/home", origin: "http://127.0.0.1:47820" }));
    expect(result).toEqual({ sessionKey: null, diagnostic: null });
  });
});

describe("versionProblem", () => {
  test("a build with no version to compare has no problem", () => {
    expect(versionProblem("0.1.0")).toBeNull();
  });
});
