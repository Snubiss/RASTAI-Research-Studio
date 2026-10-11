// vitest settings for the whole JavaScript workspace (layout: frontend-and-plugins.md section 1.1).
// Unit tests sit next to their source as <file>.test.js or <file>.test.jsx and run in Node. The include list is
// explicit so Playwright's tests/e2e/*.spec.js and anything under spike/ or .dev/ are never picked up.

import { defineConfig } from "vitest/config";

const TEST_FILES = [
  "apps/*/src/**/*.test.{js,jsx}",
  "packages/*/src/**/*.test.{js,jsx}",
  "plugins/*/ui/src/**/*.test.{js,jsx}",
  "tests/workspace/**/*.test.js",
];

export default defineConfig({
  test: {
    include: TEST_FILES,
  },
});
