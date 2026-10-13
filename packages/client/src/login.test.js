// Sign-in (layout frontend-and-plugins.md 2.6, 3.5): the cases of fixtures/ui/fragments.json, which apps/web's
// router test reads too, and the login-code exchange against the fake daemon.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { exchangeLoginCode, readLoginCode, removeLoginCodeFromUrl } from "./login.js";
import { createFakeDaemon } from "./testing/fake-daemon.js";

/**
 * One case of fixtures/ui/fragments.json.
 * @typedef {object} FragmentCase
 * @property {string} fragment
 * @property {string} search
 * @property {string | null} login_code
 * @property {string | null} fragment_after_removal
 */

const FRAGMENTS_FILE = new URL("../../../fixtures/ui/fragments.json", import.meta.url);
/** @type {FragmentCase[]} */
const FRAGMENT_CASES = JSON.parse(readFileSync(FRAGMENTS_FILE, "utf8")).cases;
const PAGE_PATH = "/";
const LOGIN_CODE = "4f".repeat(32);
const SESSION_KEY = "9e".repeat(32);

afterEach(() => {
  vi.unstubAllGlobals();
});

/**
 * Stands in for the browser window: its location and a history whose replaceState is recorded.
 * @param {FragmentCase} fragmentCase
 */
function stubWindow(fragmentCase) {
  const replaceState = vi.fn();
  vi.stubGlobal("window", {
    location: { hash: fragmentCase.fragment, pathname: PAGE_PATH, search: fragmentCase.search },
    history: { state: null, replaceState },
  });
  return replaceState;
}

describe("the fragment cases", () => {
  test("the fixture has the cases layout 3.5 names", () => {
    const fragments = FRAGMENT_CASES.map((fragmentCase) => fragmentCase.fragment);
    expect(fragments).toContain("#login=abc");
    expect(fragments).toContain("#/open?path=x&login=abc");
    expect(FRAGMENT_CASES).toContainEqual(expect.objectContaining({ fragment: "", search: "?login=abc" }));
  });

  test.each(FRAGMENT_CASES)("readLoginCode reads $login_code from '$fragment' '$search'", (fragmentCase) => {
    expect(readLoginCode({ hash: fragmentCase.fragment })).toBe(fragmentCase.login_code);
  });

  test.each(FRAGMENT_CASES)("removeLoginCodeFromUrl turns '$fragment' into $fragment_after_removal", (fragmentCase) => {
    const replaceState = stubWindow(fragmentCase);
    removeLoginCodeFromUrl();
    if (fragmentCase.fragment_after_removal === null) {
      expect(replaceState).not.toHaveBeenCalled();
      return;
    }
    expect(replaceState).toHaveBeenCalledTimes(1);
    expect(replaceState.mock.calls[0][2]).toBe(PAGE_PATH + fragmentCase.search + fragmentCase.fragment_after_removal);
  });

  test("after removal the fragment has no login code to read", () => {
    for (const fragmentCase of FRAGMENT_CASES) {
      if (fragmentCase.fragment_after_removal !== null) {
        expect(readLoginCode({ hash: fragmentCase.fragment_after_removal })).toBeNull();
      }
    }
  });
});

describe("exchangeLoginCode", () => {
  /** @type {import("./testing/fake-daemon.js").FakeDaemon} */
  let daemon;

  beforeEach(() => {
    daemon = createFakeDaemon();
    daemon.loginCodes[LOGIN_CODE] = SESSION_KEY;
    vi.stubGlobal("fetch", daemon.fetch);
  });

  test("returns the session key and sends the code in the body, with no credential header", async () => {
    const answer = await exchangeLoginCode(daemon.baseUrl, LOGIN_CODE);
    expect(answer).toEqual({ sessionKey: SESSION_KEY, diagnostic: null });
    const request = daemon.requests[0];
    expect([request.method, request.path]).toEqual(["POST", "/api/login"]);
    expect(request.headers.has("Authorization")).toBe(false);
    expect(request.headers.has("Rastai-Session-Key")).toBe(false);
  });

  test("a used or unknown code is RRS-U0002", async () => {
    await exchangeLoginCode(daemon.baseUrl, LOGIN_CODE);
    const second = await exchangeLoginCode(daemon.baseUrl, LOGIN_CODE);
    expect(second.sessionKey).toBeNull();
    expect(second.diagnostic?.code).toBe("RRS-U0002");
  });

  test("an unreachable daemon is RRS-U0001", async () => {
    daemon.reachable = false;
    const answer = await exchangeLoginCode(daemon.baseUrl, LOGIN_CODE);
    expect(answer.diagnostic?.code).toBe("RRS-U0001");
  });
});
