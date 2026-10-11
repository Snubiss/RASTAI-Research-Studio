// The generated types as a caller sees them (layout rust-control-plane.md 4.9). `just check-js` type-checks this
// file, so the typed constants below prove the typedefs are the shapes rastai-formats writes: a full header and a
// minimal one fit `SharedHeader`, and each `@ts-expect-error` line proves a wrong shape is refused (tsc fails if the
// line it marks has no error). The tests then check what the package holds at run time.

import { expect, test } from "vitest";

import * as formats from "./index.js";

/** @typedef {import("./index.js").SharedHeader} SharedHeader */

/** Every header field, as rastai-formats writes it (contracts 5.1). */
/** @type {SharedHeader} */
const FULL_HEADER = {
  $schema: "https://schemas.rastai.dev/rastai.model/1.json",
  type: "rastai.model",
  schema_version: 1,
  created_with: "rastai 0.1.0",
  id: "model_01JB3Z8Q4K7F1V0C2D9E6H5M",
  content_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000",
  derived_from: [
    "model_01JB3Z8Q4K7F1V0C2D9E6H5L@blake3:1111111111111111111111111111111111111111111111111111111111111111",
  ],
  authors: ["Ada"],
  license: "Apache-2.0",
  source_url: "https://example.org/model",
  requires: { psychology: "^1.0.0" },
};

/** Only the four required fields. */
/** @type {SharedHeader} */
const MINIMAL_HEADER = {
  $schema: "https://schemas.rastai.dev/rastai.lock/1.json",
  type: "rastai.lock",
  schema_version: 1,
  id: "lock_1",
};

/** @type {SharedHeader} */
const WRONG_VERSION = {
  $schema: "s",
  type: "rastai.model",
  // @ts-expect-error schema_version is a number, never a string
  schema_version: "1",
  id: "m",
};

// @ts-expect-error id is required
/** @type {SharedHeader} */ const NO_ID = { $schema: "s", type: "rastai.model", schema_version: 1 };

/** @type {SharedHeader} */
const UNKNOWN_KEY = {
  $schema: "s",
  type: "rastai.model",
  schema_version: 1,
  id: "m",
  // @ts-expect-error a key the header doesn't have
  kind: "x",
};

/** @type {SharedHeader} */
const WRONG_REQUIRES = {
  $schema: "s",
  type: "rastai.model",
  schema_version: 1,
  id: "m",
  // @ts-expect-error requires maps a plugin id to a semver range string
  requires: { psychology: 1 },
};

test("the shared header round-trips through JSON with every field kept", () => {
  for (const header of [FULL_HEADER, MINIMAL_HEADER]) {
    /** @type {SharedHeader} */
    const readBack = JSON.parse(JSON.stringify(header));
    expect(readBack).toEqual(header);
  }
});

test("the package holds types only, so importing it costs nothing at run time", () => {
  expect(Object.keys(formats)).toEqual([]);
});

test("the shapes the type check refuses are kept out of the run", () => {
  // Referenced so the deliberately wrong constants above count as used.
  expect([WRONG_VERSION, NO_ID, UNKNOWN_KEY, WRONG_REQUIRES]).toHaveLength(4);
});
