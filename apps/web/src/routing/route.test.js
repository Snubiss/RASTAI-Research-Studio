// The route grammar (layout frontend-and-plugins.md 2.6). The fragment table is shared with @rastai/client's login
// test (fixtures/ui/fragments.json), so the router and the sign-in can't disagree about what a fragment means.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";
// @ts-expect-error -- as above
import { dirname, resolve } from "node:path";
// @ts-expect-error -- as above
import { fileURLToPath } from "node:url";

import { describe, expect, test } from "vitest";

import { formatRoute, parseRoute, sectionFragment } from "./route.js";

const here = dirname(fileURLToPath(import.meta.url));
const FRAGMENT_TABLE = JSON.parse(readFileSync(resolve(here, "../../../../fixtures/ui/fragments.json"), "utf8"));

describe("parseRoute", () => {
  test("a section's own fragment is that section with no path", () => {
    expect(parseRoute("#/home")).toEqual({ sectionId: "home", path: [], query: {} });
  });

  test("the segments after the section are the path, decoded", () => {
    expect(parseRoute("#/inference/chat/conv%20one")).toEqual({
      sectionId: "inference",
      path: ["chat", "conv one"],
      query: {},
    });
  });

  test("a query is read with URLSearchParams and a reserved login key is never a route key", () => {
    expect(parseRoute("#/open?path=%2Fhome%2Fana%2Fmy%20notes.rrsmodel&login=abc")).toEqual({
      sectionId: "open",
      path: [],
      query: { path: "/home/ana/my notes.rrsmodel" },
    });
  });

  test("a bare query, or an empty fragment, is Home", () => {
    expect(parseRoute("#login=abc").sectionId).toBe("home");
    expect(parseRoute("").sectionId).toBe("home");
    expect(parseRoute("#").sectionId).toBe("home");
  });

  test("a segment that isn't valid percent-encoding is kept as written", () => {
    expect(parseRoute("#/inference/%E0%A4%A").path).toEqual(["%E0%A4%A"]);
  });

  test("the fixture table: every fragment that survives sign-in parses to a section", () => {
    for (const fixture of FRAGMENT_TABLE.cases) {
      if (fixture.fragment_after_removal === null) {
        continue;
      }
      const route = parseRoute(fixture.fragment_after_removal);
      expect(route.sectionId, fixture.fragment_after_removal).not.toBe("");
      expect(route.query, fixture.fragment_after_removal).not.toHaveProperty("login");
    }
  });
});

describe("formatRoute", () => {
  test("writes the fragment with its query percent-encoded, and reads back to the same route", () => {
    const route = { sectionId: "open", path: [], query: { path: "/home/ana/my notes.rrsmodel" } };
    const fragment = formatRoute(route);
    expect(fragment).toBe("#/open?path=%2Fhome%2Fana%2Fmy%20notes.rrsmodel");
    expect(parseRoute(fragment)).toEqual(route);
  });

  test("never writes the login key, even if a caller passes one", () => {
    expect(formatRoute({ sectionId: "home", path: [], query: { login: "abc" } })).toBe("#/home");
  });

  test("the fixture's open route is what the router writes back", () => {
    const fixture = FRAGMENT_TABLE.cases.find((/** @type {any} */ item) => item.fragment === "#/open?path=x&login=abc");
    expect(formatRoute(parseRoute(fixture.fragment_after_removal))).toBe(fixture.fragment_after_removal);
  });
});

describe("sectionFragment", () => {
  test("a section's page is #/<section_id>", () => {
    expect(sectionFragment("inference")).toBe("#/inference");
  });
});
