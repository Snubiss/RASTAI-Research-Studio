// Golden messages (layout: rust-node-gateway-proto.md section 3.5). Every message of rastai.common.v1 has a
// golden in fixtures/proto-golden/common/: <Name>.binpb decodes and encodes back to exactly the same bytes, the
// <Name>.json encodes to those bytes and the bytes print as that JSON. The Rust and Python tests compare against the
// same .binpb files, so the three encoders agree byte for byte.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync, readdirSync } from "node:fs";

import { createRegistry, fromBinary, fromJson, toBinary, toJson } from "@bufbuild/protobuf";
import { describe, expect, test } from "vitest";

import {
  file_rastai_common_v1_blob,
  file_rastai_common_v1_diagnostic,
  file_rastai_common_v1_hardware,
  file_rastai_common_v1_resources,
  file_rastai_common_v1_states,
} from "./index.js";

/** @typedef {import("@bufbuild/protobuf").DescMessage} DescMessage */

const PACKAGE_NAME = "rastai.common.v1";
// The files of the package. A new .proto file adds its line here; until it does, the first test below fails,
// because the new file's goldens name messages this list doesn't have.
const PACKAGE_FILES = [
  file_rastai_common_v1_blob,
  file_rastai_common_v1_diagnostic,
  file_rastai_common_v1_hardware,
  file_rastai_common_v1_resources,
  file_rastai_common_v1_states,
];
const GOLDEN_FOLDER = new URL("../../../fixtures/proto-golden/common/", import.meta.url);
const BINARY_SUFFIX = ".binpb";
const JSON_SUFFIX = ".json";
// The goldens use proto field names (snake_case), as documents store wire types (contracts 4).
const JSON_OPTIONS = { useProtoFieldName: true };
// Like prost, drop fields the generated code doesn't know, so code that lags behind the .proto changes the bytes.
const BINARY_OPTIONS = { readUnknownFields: false };

/**
 * @param {string} messageName
 * @param {string} suffix
 */
function goldenUrl(messageName, suffix) {
  return new URL(`${messageName}${suffix}`, GOLDEN_FOLDER);
}

/** @param {string} messageName */
function readGoldenBytes(messageName) {
  return new Uint8Array(readFileSync(goldenUrl(messageName, BINARY_SUFFIX)));
}

/** @param {string} messageName */
function readGoldenJson(messageName) {
  return JSON.parse(readFileSync(goldenUrl(messageName, JSON_SUFFIX), "utf8"));
}

/**
 * The message names the golden folder has: one per <Name>.json.
 * @returns {string[]}
 */
function goldenMessageNames() {
  /** @type {string[]} */
  const fileNames = readdirSync(GOLDEN_FOLDER);
  const jsonNames = fileNames.filter((name) => name.endsWith(JSON_SUFFIX));
  return jsonNames.map((name) => name.slice(0, -JSON_SUFFIX.length)).sort();
}

/**
 * The fields a golden leaves out, as dotted paths. A golden sets every field, so a field added to the .proto
 * without a golden update shows up here. toJson leaves out unset fields, so the JSON says what was set.
 * @param {DescMessage} schema
 * @param {Record<string, any>} json
 * @param {string} prefix
 * @returns {string[]}
 */
function missingFields(schema, json, prefix) {
  /** @type {string[]} */
  const missing = [];
  for (const field of schema.fields) {
    const path = `${prefix}${field.name}`;
    const value = json[field.name];
    if (value === undefined) {
      missing.push(path);
    } else if (field.fieldKind === "message") {
      missing.push(...missingFields(field.message, value, `${path}.`));
    } else if (field.fieldKind === "list" && field.listKind === "message") {
      for (const [index, item] of value.entries()) {
        missing.push(...missingFields(field.message, item, `${path}[${index}].`));
      }
    }
  }
  return missing;
}

const registry = createRegistry(...PACKAGE_FILES);
const messageNames = PACKAGE_FILES.flatMap((file) => file.messages.map((message) => message.name)).sort();

/** @param {string} messageName */
function schemaOf(messageName) {
  const schema = registry.getMessage(`${PACKAGE_NAME}.${messageName}`);
  if (schema === undefined) {
    throw new Error(`${messageName} isn't a message of ${PACKAGE_NAME}`);
  }
  return schema;
}

describe(`${PACKAGE_NAME} goldens`, () => {
  test("every message has a golden and every golden names a message", () => {
    expect(messageNames).toContain("Diagnostic");
    expect(goldenMessageNames()).toEqual(messageNames);
  });

  test.each(messageNames)("%s round-trips byte-identical", (messageName) => {
    const schema = schemaOf(messageName);
    const goldenBytes = readGoldenBytes(messageName);
    const message = fromBinary(schema, goldenBytes, BINARY_OPTIONS);
    expect(toBinary(schema, message)).toEqual(goldenBytes);
  });

  test.each(messageNames)("%s JSON and binary goldens agree", (messageName) => {
    const schema = schemaOf(messageName);
    const goldenBytes = readGoldenBytes(messageName);
    const goldenJson = readGoldenJson(messageName);
    expect(toBinary(schema, fromJson(schema, goldenJson))).toEqual(goldenBytes);
    expect(toJson(schema, fromBinary(schema, goldenBytes), JSON_OPTIONS)).toEqual(goldenJson);
  });

  test.each(messageNames)("%s golden sets every field", (messageName) => {
    expect(missingFields(schemaOf(messageName), readGoldenJson(messageName), "")).toEqual([]);
  });
});
