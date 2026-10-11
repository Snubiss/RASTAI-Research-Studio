// Smoke test for the JavaScript workspace (M1-04): vitest runs inside `just check`, and the checker settings that
// every package's jsconfig.json inherits stay the ones ADR-0036 decided. Each test names why its setting matters.

import { expect, test } from "vitest";
import baseConfig from "../../jsconfig.base.json" with { type: "json" };
import rootConfig from "../../jsconfig.json" with { type: "json" };

const options = baseConfig.compilerOptions;

test("checkJs is on, so the JSDoc types in plain .js files are checked", () => {
  expect(options.allowJs).toBe(true);
  expect(options.checkJs).toBe(true);
});

test("noEmit is on: tsc is a checker only and never writes JavaScript", () => {
  expect(options.noEmit).toBe(true);
});

test("strict is on, so a wrong field, a missing argument or a possible null fails the check", () => {
  expect(options.strict).toBe(true);
});

test("JSX compiles the React way, matching Vite's esbuild transform", () => {
  expect(options.jsx).toBe("react-jsx");
});

test("the workspace config extends the base, so there is one set of settings", () => {
  expect(rootConfig.extends).toBe("./jsconfig.base.json");
});
