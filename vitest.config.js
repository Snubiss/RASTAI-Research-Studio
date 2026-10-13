// vitest settings for the whole JavaScript workspace (layout: frontend-and-plugins.md sections 1.1 and 4.5).
// Unit tests sit next to their source as <file>.test.js or <file>.test.jsx and run in Node. Component tests are
// <file>.browser.test.jsx and run in real browser engines, Chromium and WebKit, through Playwright (no jsdom):
// WebKit is the engine of the Tauri window on macOS and Linux. `tools/ci/install-dev-tools browsers` installs the
// pinned builds. The include lists are explicit so Playwright's tests/e2e/*.spec.js and anything under spike/ or
// .dev/ are never picked up.

import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

const NODE_TEST_FILES = [
  "apps/*/src/**/*.test.{js,jsx}",
  "packages/*/src/**/*.test.{js,jsx}",
  "plugins/*/ui/src/**/*.test.{js,jsx}",
  "tests/workspace/**/*.test.js",
];
const BROWSER_TEST_FILES = [
  "apps/*/src/**/*.browser.test.{js,jsx}",
  "packages/*/src/**/*.browser.test.{js,jsx}",
  "plugins/*/ui/src/**/*.browser.test.{js,jsx}",
];
// Screenshots compare two renders taken in the same run, never a stored picture, so a fixed viewport is enough.
const BROWSER_VIEWPORT = { width: 1024, height: 768 };

export default defineConfig({
  test: {
    projects: [
      {
        test: {
          name: "node",
          include: NODE_TEST_FILES,
          exclude: BROWSER_TEST_FILES,
        },
      },
      {
        test: {
          name: "browser",
          include: BROWSER_TEST_FILES,
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            viewport: BROWSER_VIEWPORT,
            instances: [{ browser: "chromium" }, { browser: "webkit" }],
          },
        },
      },
    ],
  },
});
