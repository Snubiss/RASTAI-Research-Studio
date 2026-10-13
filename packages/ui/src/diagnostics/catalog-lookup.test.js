// The catalog text for a Diagnostic (layout frontend-and-plugins.md 2.7): placeholders read back out of a filled
// message, the wire severity, and the fallbacks for a code the catalog doesn't know.

import { CATALOG } from "@rastai/catalog";
import { describe, expect, test } from "vitest";

import { catalogEntry, diagnosticText, fillPlaceholders, placeholderValues } from "./catalog-lookup.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

/**
 * @param {Partial<Diagnostic>} fields
 * @returns {Diagnostic}
 */
function diagnostic(fields) {
  return /** @type {Diagnostic} */ ({
    $typeName: "rastai.common.v1.Diagnostic",
    code: "RRS-U0001",
    severity: 1,
    message: "The daemon at http://127.0.0.1:47820 is not reachable.",
    path: "",
    fixText: "",
    fixCommandId: "",
    fixArgumentsJson: "",
    ...fields,
  });
}

describe("placeholders", () => {
  test("reads each value back out of a filled message", () => {
    expect(
      placeholderValues(
        "The web app is version {app_version}, but the daemon is version {daemon_version}.",
        "The web app is version 0.1.0, but the daemon is version 0.2.0.",
      ),
    ).toEqual({
      app_version: "0.1.0",
      daemon_version: "0.2.0",
    });
  });

  test("a message that doesn't fit the template, or a template without placeholders, gives no values", () => {
    expect(placeholderValues("The daemon at {address} is not reachable.", "Something else.")).toEqual({});
    expect(placeholderValues("You were signed out.", "You were signed out.")).toEqual({});
  });

  test("regex characters in the template are literal", () => {
    expect(placeholderValues("Size (max {limit}) [bytes]?", "Size (max 10) [bytes]?")).toEqual({ limit: "10" });
  });

  test("fills what it has and leaves the rest visible", () => {
    expect(fillPlaceholders("{file_name} is {size}.", { file_name: "a.csv" })).toBe("a.csv is {size}.");
  });
});

describe("diagnosticText", () => {
  test("uses the catalog entry and the wire severity", () => {
    const entry = catalogEntry("RRS-U0001");
    const text = diagnosticText(diagnostic({ severity: 2 }));
    expect(text).toEqual({
      severity: "warning",
      title: entry?.title,
      basicText: entry?.basic_text,
      fixText: entry?.fix_text,
      inCatalog: true,
    });
  });

  test("the diagnostic's own fix text wins over the catalog's", () => {
    expect(diagnosticText(diagnostic({ fixText: "Run rastai open." })).fixText).toBe("Run rastai open.");
  });

  test("an unset severity falls back to the catalog's", () => {
    const entry = CATALOG.find((item) => item.severity === "info");
    expect(diagnosticText(diagnostic({ code: entry?.code, severity: 0 })).severity).toBe("info");
  });

  test("an unknown code shows the message under the code", () => {
    const text = diagnosticText(diagnostic({ code: "PLG-example-0001", message: "It failed.", severity: 0 }));
    expect(text).toEqual({
      severity: "error",
      title: "PLG-example-0001",
      basicText: "It failed.",
      fixText: "",
      inCatalog: false,
    });
  });

  test("every catalog entry has the Basic text DiagnosticView shows", () => {
    for (const entry of CATALOG) {
      expect(entry.basic_text, entry.code).not.toBe("");
    }
  });
});
