// copy-as writes every line of fixtures/command-golden/copy-as.json. The same file is parsed back by the CLI's tests and
// checked against the names by naming_round_trip.rs, so a change here that the golden file doesn't show is a drift.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { copyAsFormats, formatCli, formatJavaScript, formatMcp, formatPython } from "./copy-as.js";

const GOLDEN_PATH = new URL("../../../../fixtures/command-golden/copy-as.json", import.meta.url);
const golden = JSON.parse(readFileSync(GOLDEN_PATH, "utf8"));

describe("copy as, against the golden file", () => {
  for (const example of golden.examples) {
    test(`${example.command_id} is written as the golden file has it`, () => {
      const formats = copyAsFormats(example.command_id, example.arguments);
      expect(formats.cli).toBe(example.cli);
      expect(formats.python).toBe(example.python);
      expect(formats.javascript).toBe(example.javascript);
      expect(JSON.parse(formats.mcp)).toEqual(example.mcp);
    });
  }
});

describe("the rules behind the golden lines", () => {
  test("a true boolean is its flag alone and a false one its --no- flag", () => {
    expect(formatCli("token.list", { include_revoked: true })).toBe("rastai token list --include-revoked");
    expect(formatCli("command.list", { basic_only: false })).toBe("rastai command list --no-basic-only");
  });

  test("a list repeats its flag once per element, in order", () => {
    expect(formatCli("token.create", { scopes: ["training", "eval"] })).toBe(
      "rastai token create --scopes training --scopes eval",
    );
  });

  test("text with a quote is shell-quoted, with the quote written as '\\''", () => {
    expect(formatCli("token.create", { name: "ci bot's token" })).toBe(
      "rastai token create --name 'ci bot'\\''s token'",
    );
  });

  test("the Python and JavaScript lines take an empty argument object", () => {
    expect(formatPython("system.status", {})).toBe('rastai.connect().execute("system.status", {})');
    expect(formatJavaScript("system.status", {})).toBe('await client.execute("system.status", {})');
  });

  test("the MCP name writes each dot as two underscores", () => {
    expect(JSON.parse(formatMcp("token.login_code.create", {})).name).toBe("token__login_code__create");
  });
});
