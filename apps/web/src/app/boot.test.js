// Sign-in (layout frontend-and-plugins.md 8.3): a window with no login code in its fragment has no session, and asks
// no one anything. The code exchange itself is @rastai/client's, tested there.

import { describe, expect, test } from "vitest";

import { pluginDiagnostic } from "@rastai/plugin";

import { noSessionScreen, signIn } from "./boot.js";
import { versionProblem } from "./daemon.js";

describe("signIn", () => {
  test("without a login code in the fragment there is no session key and no request", async () => {
    const result = await signIn(/** @type {any} */ ({ hash: "#/home", origin: "http://127.0.0.1:47820" }));
    expect(result).toEqual({ sessionKey: null, diagnostic: null });
  });
});

describe("noSessionScreen", () => {
  test("a daemon that can't be reached at sign-in is the offline screen (RRS-U0001)", () => {
    expect(noSessionScreen(pluginDiagnostic("RRS-U0001", { address: "http://127.0.0.1:47820" }))).toBe("offline");
  });

  test("a rejected code (RRS-U0002), and no diagnostic at all, are the signed-out screen", () => {
    expect(noSessionScreen(pluginDiagnostic("RRS-U0002", {}))).toBe("signed_out");
    expect(noSessionScreen(null)).toBe("signed_out");
  });
});

describe("versionProblem", () => {
  test("a build with no version to compare has no problem", () => {
    expect(versionProblem("0.1.0")).toBeNull();
  });

  test("a daemon of another version than this build is RRS-U0004, naming both versions", () => {
    const problem = versionProblem("0.2.0", "0.1.0");
    expect(problem?.code).toBe("RRS-U0004");
    expect(problem?.message).toContain("0.2.0");
    expect(problem?.message).toContain("0.1.0");
  });

  test("the same version has no problem", () => {
    expect(versionProblem("0.1.0", "0.1.0")).toBeNull();
  });
});
