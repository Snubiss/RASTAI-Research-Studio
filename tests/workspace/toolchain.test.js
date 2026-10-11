// Smoke test for the pinned JavaScript toolchain (M1-04). Every machine checks with the same Node.js: package.json
// names one exact version, and `engineStrict` in pnpm-workspace.yaml makes `just env-js` (and so every JS recipe)
// refuse any other Node.js. The justfile header gives that version's download and SHA-256.

import { expect, test } from "vitest";
import packageFile from "../../package.json" with { type: "json" };

// One exact version: digits and dots only, never a range such as ">=24" or "24.x".
const EXACT_VERSION = /^\d+\.\d+\.\d+$/;

// The workspace file, read as plain text: a line check needs no YAML parser. Lines split on "\r\n" too, so a
// Windows checkout with CRLF endings passes the same way.
const WORKSPACE_FILE_URL = new URL("../../pnpm-workspace.yaml", import.meta.url);

// The workspace has no Node.js type definitions (@types/node is not a spec §27 dependency), so a static
// `import ... from "node:fs"` fails `just check-js`. Importing it by a name held in a constant skips that lookup;
// the JSDoc below gives the one function used here its type instead.
const FILE_SYSTEM_MODULE = "node:fs";

/** @type {{ readFileSync: (path: URL, encoding: string) => string }} */
const fileSystem = await import(FILE_SYSTEM_MODULE);

// The line that makes pnpm enforce the pin. Without it pnpm only warns, and a wrong Node.js still installs.
const ENGINE_STRICT_LINE = "engineStrict: true";

test("package.json pins Node.js to one exact version, not a range", () => {
  expect(packageFile.engines.node).toMatch(EXACT_VERSION);
});

test("pnpm-workspace.yaml turns on engineStrict, so a different Node.js is refused", () => {
  const workspaceLines = fileSystem.readFileSync(WORKSPACE_FILE_URL, "utf8").split(/\r?\n/);
  expect(workspaceLines).toContain(ENGINE_STRICT_LINE);
});
