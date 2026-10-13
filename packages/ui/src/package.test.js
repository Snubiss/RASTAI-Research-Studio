// The package's boundaries (layout frontend-and-plugins.md 2.4, 2.10 and 4.4): the security components come only
// from the @rastai/ui/security subpath, there is no BasicOnly, the stylesheets read only variables applyThemeTokens
// writes, and no source file turns text into HTML.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync, readdirSync } from "node:fs";

import { describe, expect, test } from "vitest";

import * as ui from "./index.js";
import * as security from "./security/index.js";
import { lockedVariables, themeVariables } from "./tokens/apply-theme.js";
import { builtInTheme } from "./tokens/resolve-tokens.js";
import packageFile from "../package.json" with { type: "json" };

const SOURCE_FOLDER = new URL("./", import.meta.url);
const STYLESHEETS = ["styles/base.css", "styles/components.css"];
const CSS_VARIABLE_USE = /var\((--rr-[a-z0-9-]+)/g;
const SECURITY_COMPONENTS = ["EgressBadge", "PermissionList", "SignedOutScreen", "TrustPrompt"];
// Written as joined parts, so this file doesn't contain the words it looks for.
const FORBIDDEN_WORDS = ["inner" + "HTML", "dangerously" + "SetInnerHTML", "outer" + "HTML", "insertAdjacent" + "HTML"];

/**
 * Every hand-written file under src/, as a path relative to src/.
 * @param {string} folder   relative to src/, "" for src/ itself
 * @returns {string[]}
 */
function sourceFiles(folder) {
  /** @type {string[]} */
  const files = [];
  for (const entry of readdirSync(new URL(folder, SOURCE_FOLDER), { withFileTypes: true })) {
    const path = `${folder}${entry.name}`;
    if (entry.isDirectory()) {
      files.push(...sourceFiles(`${path}/`));
    } else {
      files.push(path);
    }
  }
  return files;
}

describe("exports", () => {
  test("the security components come only from the security subpath", () => {
    expect(Object.keys(security).sort()).toEqual(SECURITY_COMPONENTS);
    for (const name of SECURITY_COMPONENTS) {
      expect(Object.keys(ui)).not.toContain(name);
    }
    expect(packageFile.exports["./security"]).toBe("./src/security/index.js");
  });

  test("Advanced can only add: there is AdvancedOnly and no BasicOnly", () => {
    expect(Object.keys(ui)).toContain("AdvancedOnly");
    expect(Object.keys(ui).filter((name) => name.toLowerCase().includes("basiconly"))).toEqual([]);
  });
});

describe("stylesheets", () => {
  test("read only the variables applyThemeTokens writes", () => {
    const tokens = builtInTheme("light", { density: "comfortable", reducedMotion: false, colorblind: false });
    const written = new Set([...themeVariables(tokens), ...lockedVariables(tokens)].map(([name]) => name));
    for (const stylesheet of STYLESHEETS) {
      const text = readFileSync(new URL(stylesheet, SOURCE_FOLDER), "utf8");
      const used = [...text.matchAll(CSS_VARIABLE_USE)].map((match) => match[1]);
      expect(used.length, stylesheet).toBeGreaterThan(0);
      expect(
        used.filter((name) => !written.has(name)),
        stylesheet,
      ).toEqual([]);
    }
  });
});

describe("no HTML from strings", () => {
  test("no source file builds HTML from strings", () => {
    for (const file of sourceFiles("")) {
      const text = readFileSync(new URL(file, SOURCE_FOLDER), "utf8");
      for (const word of FORBIDDEN_WORDS) {
        expect(text.includes(word), `${file} uses ${word}`).toBe(false);
      }
    }
  });
});
