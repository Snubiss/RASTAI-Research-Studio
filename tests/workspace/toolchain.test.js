// Smoke test for the pinned JavaScript toolchain (M1-04). Every machine checks with the same Node.js: package.json
// names one exact version, and `engineStrict` in pnpm-workspace.yaml makes `just env-js` (and so every JS recipe)
// refuse any other Node.js. The justfile header gives that version's download and SHA-256.

import { expect, test } from "vitest";
import packageFile from "../../package.json" with { type: "json" };

// One exact version: digits and dots only, never a range such as ">=24" or "24.x".
const EXACT_VERSION = /^\d+\.\d+\.\d+$/;

test("package.json pins Node.js to one exact version, not a range", () => {
  expect(packageFile.engines.node).toMatch(EXACT_VERSION);
});
