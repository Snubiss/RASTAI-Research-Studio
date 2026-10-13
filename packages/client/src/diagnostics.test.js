// clientDiagnostic (layout frontend-and-plugins.md 3.1): its messages are the catalog's, and its placeholders fill.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { Severity } from "@rastai/proto";
import { expect, test } from "vitest";

import { CLIENT_MESSAGES, clientDiagnostic, diagnosticFromRefusalJson } from "./diagnostics.js";

// The client may not import @rastai/catalog (contracts 2.3), so the test reads the catalog's source file.
const RRS_U_CATALOG = new URL("../../../catalog/errors/RRS-U.json", import.meta.url);

/** @type {Array<{ code: string, severity: string, message: string }>} */
const CATALOG_ENTRIES = JSON.parse(readFileSync(RRS_U_CATALOG, "utf8"));

test.each(Object.keys(CLIENT_MESSAGES))("%s has the catalog's message and is an error", (code) => {
  const entry = CATALOG_ENTRIES.find((candidate) => candidate.code === code);
  expect(entry, `${code} is in catalog/errors/RRS-U.json`).toBeDefined();
  expect(CLIENT_MESSAGES[code]).toBe(entry?.message);
  expect(entry?.severity).toBe("error");
});

test("clientDiagnostic fills the placeholders and is an error with the code", () => {
  const diagnostic = clientDiagnostic("RRS-U0001", { address: "http://127.0.0.1:47820" });
  expect(diagnostic.code).toBe("RRS-U0001");
  expect(diagnostic.severity).toBe(Severity.ERROR);
  expect(diagnostic.message).toBe("The daemon at http://127.0.0.1:47820 is not reachable.");
});

test("a placeholder without a value stays visible", () => {
  expect(clientDiagnostic("RRS-U0006").message).toBe("{file_name} is too large to upload from the browser.");
});

test("a refusal body from a plain HTTP path becomes the daemon's Diagnostic", () => {
  const json = { code: "RRS-C1004", severity: "error", message: "Origin not allowed", fix_text: "Open it again." };
  const diagnostic = diagnosticFromRefusalJson(json);
  expect([diagnostic?.code, diagnostic?.severity, diagnostic?.fixText]).toEqual([
    "RRS-C1004",
    Severity.ERROR,
    "Open it again.",
  ]);
  expect(diagnosticFromRefusalJson({ status: "ok" })).toBeNull();
});
