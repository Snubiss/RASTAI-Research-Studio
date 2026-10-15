// copy-as writes every line of fixtures/command-golden/copy-as.json. The same file is parsed back by the CLI's tests and
// checked against the names by naming_round_trip.rs, so a change here that the golden file doesn't show is a drift.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import { copyAsFormats, formatCli, formatJavaScript, formatMcp, formatPython } from "./copy-as.js";

const GOLDEN_PATH = new URL("../../../../fixtures/command-golden/copy-as.json", import.meta.url);
const golden = JSON.parse(readFileSync(GOLDEN_PATH, "utf8"));
const COMMAND_SCHEMAS = new URL("../../../../schemas/commands/", import.meta.url);
// A value that appears nowhere else, so finding it in a copied line means the secret leaked.
const TYPED_SECRET = "hf_live_TYPED_SECRET_31415";

/**
 * The arguments schema of a checked-in command schema, as the daemon sends it in arguments_schema_json.
 * @param {string} commandId
 * @returns {string}
 */
function argumentsSchemaJson(commandId) {
  const file = new URL(`rastai.command.${commandId}/1.json`, COMMAND_SCHEMAS);
  return JSON.stringify(JSON.parse(readFileSync(file, "utf8")).$defs.arguments);
}

describe("copy as, against the golden file", () => {
  for (const example of golden.examples) {
    test(`${example.command_id} is written as the golden file has it`, () => {
      // The golden commands have no secret argument, so no schema is read for them.
      const formats = copyAsFormats(example.command_id, example.arguments, "");
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

describe("secret arguments are left out, in every format", () => {
  test("secret.set never copies the typed value, and keeps its name", () => {
    const formats = copyAsFormats(
      "secret.set",
      { name: "hf_token", value: TYPED_SECRET },
      argumentsSchemaJson("secret.set"),
    );
    for (const text of Object.values(formats)) {
      expect(text).not.toContain(TYPED_SECRET);
    }
    expect(formats.cli).toBe("rastai secret set --name hf_token");
    expect(formats.python).toBe('rastai.connect().execute("secret.set", {"name": "hf_token"})');
    expect(formats.javascript).toBe('await client.execute("secret.set", { name: "hf_token" })');
    expect(JSON.parse(formats.mcp)).toEqual({ name: "secret__set", arguments: { name: "hf_token" } });
  });

  test("hf.login never copies the typed token", () => {
    const formats = copyAsFormats("hf.login", { token: TYPED_SECRET }, argumentsSchemaJson("hf.login"));
    for (const text of Object.values(formats)) {
      expect(text).not.toContain(TYPED_SECRET);
    }
    expect(formats.cli).toBe("rastai hf login");
    expect(formats.python).toBe('rastai.connect().execute("hf.login", {})');
    expect(JSON.parse(formats.mcp)).toEqual({ name: "hf__login", arguments: {} });
  });

  test("a secret that is not typed yet leaves no key behind", () => {
    const formats = copyAsFormats("hf.login", {}, argumentsSchemaJson("hf.login"));
    expect(formats.cli).toBe("rastai hf login");
    expect(formats.javascript).toBe('await client.execute("hf.login", {})');
  });

  test("the marking in the schema decides which arguments are left out, not a list of names", () => {
    const schema = JSON.stringify({
      properties: { note: { type: "string", "x-rastai-secret": true }, name: { type: "string" } },
    });
    expect(copyAsFormats("x.y", { note: TYPED_SECRET, name: "n" }, schema).cli).toBe("rastai x y --name n");
  });
});
