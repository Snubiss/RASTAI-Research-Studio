// Golden messages (layout: rust-node-gateway-proto.md section 3.5). Every message of rastai.common.v1,
// rastai.commands.v1, rastai.events.v1, rastai.node.v1 and rastai.worker.v1 has a golden in
// fixtures/proto-golden/<package>/: <Name>.binpb decodes and encodes back to exactly the same bytes, the <Name>.json
// encodes to those bytes and the bytes print as that JSON.
// The Rust and Python tests compare against the same .binpb files, so the three encoders agree byte for byte.
// The Connect JSON cases in fixtures/proto-golden/commands/connect_json/ are checked here against protobuf-es, the
// reference that the daemon's hand-written codec (crates/rastai-server/tests/connect_protocol.rs) must equal.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync, readdirSync } from "node:fs";

import {
  createRegistry,
  fromBinary,
  fromJson,
  fromJsonString,
  toBinary,
  toJson,
  toJsonString,
} from "@bufbuild/protobuf";
import { describe, expect, test } from "vitest";

import {
  CommandDescriptorSchema,
  DiagnosticSchema,
  EventTypeDescriptorSchema,
  ExecuteRequestSchema,
  ExecuteResponseSchema,
  ListCommandsRequestSchema,
  ListCommandsResponseSchema,
  Stability,
  file_rastai_commands_v1_command_service,
  file_rastai_common_v1_blob,
  file_rastai_common_v1_diagnostic,
  file_rastai_common_v1_hardware,
  file_rastai_common_v1_resources,
  file_rastai_common_v1_states,
  file_rastai_events_v1_event_record,
  file_rastai_events_v1_event_service,
  file_rastai_events_v1_payloads,
  file_rastai_node_v1_jobs,
  file_rastai_node_v1_live,
  file_rastai_node_v1_node_hub_service,
  file_rastai_node_v1_node_service,
  file_rastai_worker_v1_worker_host_service,
  file_rastai_worker_v1_worker_service,
} from "./index.js";

/** @typedef {import("@bufbuild/protobuf").DescMessage} DescMessage */
/** @typedef {import("@bufbuild/protobuf").DescFile} DescFile */

/**
 * One proto package and its golden folder. A new .proto file adds its line to `files`; until it does, the first
 * test below fails, because the new file's goldens name messages the package doesn't have.
 * @typedef {object} GoldenPackage
 * @property {string} packageName
 * @property {string} folder
 * @property {DescFile[]} files
 */

/** @type {GoldenPackage[]} */
const GOLDEN_PACKAGES = [
  {
    packageName: "rastai.common.v1",
    folder: "common",
    files: [
      file_rastai_common_v1_blob,
      file_rastai_common_v1_diagnostic,
      file_rastai_common_v1_hardware,
      file_rastai_common_v1_resources,
      file_rastai_common_v1_states,
    ],
  },
  {
    packageName: "rastai.commands.v1",
    folder: "commands",
    files: [file_rastai_commands_v1_command_service],
  },
  {
    packageName: "rastai.events.v1",
    folder: "events",
    files: [file_rastai_events_v1_event_record, file_rastai_events_v1_event_service, file_rastai_events_v1_payloads],
  },
  {
    packageName: "rastai.node.v1",
    folder: "node",
    files: [
      file_rastai_node_v1_jobs,
      file_rastai_node_v1_live,
      file_rastai_node_v1_node_hub_service,
      file_rastai_node_v1_node_service,
    ],
  },
  {
    packageName: "rastai.worker.v1",
    folder: "worker",
    files: [file_rastai_worker_v1_worker_host_service, file_rastai_worker_v1_worker_service],
  },
];
const GOLDEN_ROOT = new URL("../../../fixtures/proto-golden/", import.meta.url);
// Descriptors written by a daemon that predates the stability field (layout 3.5, "Unset stability").
const WITHOUT_STABILITY_FOLDER = "without_stability";
const BINARY_SUFFIX = ".binpb";
const JSON_SUFFIX = ".json";
// The goldens use proto field names (snake_case), as documents store wire types (contracts 4).
const JSON_OPTIONS = { useProtoFieldName: true };
// Like prost, drop fields the generated code doesn't know, so code that lags behind the .proto changes the bytes.
const BINARY_OPTIONS = { readUnknownFields: false };
// The Connect JSON cases: `encode` (a .binpb and the exact text protobuf-es writes for it), `decode` (an input it
// accepts and the text it writes back) and `refuse` (an input it refuses; the code and path are the daemon's).
const CONNECT_JSON_FOLDER = "commands/connect_json";
const CONNECT_JSON_CASES_FILE = "cases.json";
/** @type {Record<string, DescMessage>} */
const CONNECT_JSON_SCHEMAS = {
  ExecuteRequest: ExecuteRequestSchema,
  ExecuteResponse: ExecuteResponseSchema,
  Diagnostic: DiagnosticSchema,
  ListCommandsRequest: ListCommandsRequestSchema,
  ListCommandsResponse: ListCommandsResponseSchema,
  CommandDescriptor: CommandDescriptorSchema,
};

/**
 * @param {string} folder
 * @param {string} messageName
 * @param {string} suffix
 */
function goldenUrl(folder, messageName, suffix) {
  return new URL(`${folder}/${messageName}${suffix}`, GOLDEN_ROOT);
}

/**
 * @param {string} folder
 * @param {string} messageName
 */
function readGoldenBytes(folder, messageName) {
  return new Uint8Array(readFileSync(goldenUrl(folder, messageName, BINARY_SUFFIX)));
}

/**
 * @param {string} folder
 * @param {string} messageName
 */
function readGoldenJson(folder, messageName) {
  return JSON.parse(readFileSync(goldenUrl(folder, messageName, JSON_SUFFIX), "utf8"));
}

/**
 * The message names a golden folder has: one per <Name>.json. Subfolders such as without_stability/ aren't counted.
 * @param {string} folder
 * @returns {string[]}
 */
function goldenMessageNames(folder) {
  /** @type {string[]} */
  const fileNames = readdirSync(new URL(`${folder}/`, GOLDEN_ROOT));
  const jsonNames = fileNames.filter((name) => name.endsWith(JSON_SUFFIX));
  return jsonNames.map((name) => name.slice(0, -JSON_SUFFIX.length)).sort();
}

/**
 * The fields a golden leaves out, as dotted paths. A golden sets every field, so a field added to the .proto
 * without a golden update shows up here. A oneof can hold only one of its fields, so it counts as set when any
 * one of them is. toJson leaves out unset fields, so the JSON says what was set.
 * @param {DescMessage} schema
 * @param {Record<string, any>} json
 * @param {string} prefix
 * @returns {string[]}
 */
function missingFields(schema, json, prefix) {
  /** @type {string[]} */
  const missing = [];
  for (const member of schema.members) {
    if (member.kind === "oneof") {
      const setField = member.fields.find((field) => json[field.name] !== undefined);
      if (setField === undefined) {
        missing.push(`${prefix}${member.name}`);
      } else {
        missing.push(...missingInsideField(setField, json[setField.name], `${prefix}${setField.name}`));
      }
    } else if (json[member.name] === undefined) {
      missing.push(`${prefix}${member.name}`);
    } else {
      missing.push(...missingInsideField(member, json[member.name], `${prefix}${member.name}`));
    }
  }
  return missing;
}

/**
 * The fields left out inside one set field's value: a nested message, or each message in a list.
 * @param {import("@bufbuild/protobuf").DescField} field
 * @param {any} value
 * @param {string} path
 * @returns {string[]}
 */
function missingInsideField(field, value, path) {
  if (field.fieldKind === "message") {
    return missingFields(field.message, value, `${path}.`);
  }
  /** @type {string[]} */
  const missing = [];
  if (field.fieldKind === "list" && field.listKind === "message") {
    for (const [index, item] of value.entries()) {
      missing.push(...missingFields(field.message, item, `${path}[${index}].`));
    }
  }
  return missing;
}

describe.each(GOLDEN_PACKAGES)("$packageName goldens", ({ packageName, folder, files }) => {
  const registry = createRegistry(...files);
  const messageNames = files.flatMap((file) => file.messages.map((message) => message.name)).sort();

  /** @param {string} messageName */
  function schemaOf(messageName) {
    const schema = registry.getMessage(`${packageName}.${messageName}`);
    if (schema === undefined) {
      throw new Error(`${messageName} isn't a message of ${packageName}`);
    }
    return schema;
  }

  test("every message has a golden and every golden names a message", () => {
    expect(messageNames.length).toBeGreaterThan(0);
    expect(goldenMessageNames(folder)).toEqual(messageNames);
  });

  test.each(messageNames)("%s round-trips byte-identical", (messageName) => {
    const schema = schemaOf(messageName);
    const goldenBytes = readGoldenBytes(folder, messageName);
    const message = fromBinary(schema, goldenBytes, BINARY_OPTIONS);
    expect(toBinary(schema, message)).toEqual(goldenBytes);
  });

  test.each(messageNames)("%s JSON and binary goldens agree", (messageName) => {
    const schema = schemaOf(messageName);
    const goldenBytes = readGoldenBytes(folder, messageName);
    const goldenJson = readGoldenJson(folder, messageName);
    expect(toBinary(schema, fromJson(schema, goldenJson))).toEqual(goldenBytes);
    expect(toJson(schema, fromBinary(schema, goldenBytes), JSON_OPTIONS)).toEqual(goldenJson);
  });

  test.each(messageNames)("%s golden sets every field", (messageName) => {
    expect(missingFields(schemaOf(messageName), readGoldenJson(folder, messageName), "")).toEqual([]);
  });
});

describe("stability on command and event-type descriptors", () => {
  test("descriptors with stability set decode it", () => {
    const command = fromBinary(CommandDescriptorSchema, readGoldenBytes("commands", "CommandDescriptor"));
    const eventType = fromBinary(EventTypeDescriptorSchema, readGoldenBytes("events", "EventTypeDescriptor"));
    expect(command.stability).toBe(Stability.PREVIEW);
    expect(eventType.stability).toBe(Stability.PREVIEW);
  });

  // An older daemon sends descriptors without the field. They decode as STABILITY_UNSPECIFIED, which every reader
  // treats as preview (layout 3.2), and they still round-trip unchanged.
  test("descriptors without stability decode as the zero value", () => {
    const commandFolder = `commands/${WITHOUT_STABILITY_FOLDER}`;
    const eventFolder = `events/${WITHOUT_STABILITY_FOLDER}`;
    const commandBytes = readGoldenBytes(commandFolder, "CommandDescriptor");
    const eventBytes = readGoldenBytes(eventFolder, "EventTypeDescriptor");
    const command = fromBinary(CommandDescriptorSchema, commandBytes);
    const eventType = fromBinary(EventTypeDescriptorSchema, eventBytes);
    expect(command.stability).toBe(Stability.UNSPECIFIED);
    expect(eventType.stability).toBe(Stability.UNSPECIFIED);
    expect(toBinary(CommandDescriptorSchema, command)).toEqual(commandBytes);
    expect(toBinary(EventTypeDescriptorSchema, eventType)).toEqual(eventBytes);
    // The JSON beside each .binpb is the same message, so a reader of either file sees no stability field.
    expect(toJson(CommandDescriptorSchema, command, JSON_OPTIONS)).toEqual(
      readGoldenJson(commandFolder, "CommandDescriptor"),
    );
    expect(toJson(EventTypeDescriptorSchema, eventType, JSON_OPTIONS)).toEqual(
      readGoldenJson(eventFolder, "EventTypeDescriptor"),
    );
  });
});

/**
 * One case of fixtures/proto-golden/commands/connect_json/cases.json.
 * @typedef {object} ConnectJsonCase
 * @property {string} message      one of the six CommandService messages, by its short name
 * @property {string} [binary]     encode: the .binpb file, relative to the cases folder
 * @property {string} [input]      decode and refuse: the JSON text sent
 * @property {string} [json]       encode and decode: the exact text protobuf-es writes
 */

/** @returns {{encode: ConnectJsonCase[], decode: ConnectJsonCase[], refuse: ConnectJsonCase[]}} */
function readConnectJsonCases() {
  const url = new URL(`${CONNECT_JSON_FOLDER}/${CONNECT_JSON_CASES_FILE}`, GOLDEN_ROOT);
  return JSON.parse(readFileSync(url, "utf8"));
}

/** @param {string} messageName */
function connectJsonSchema(messageName) {
  const schema = CONNECT_JSON_SCHEMAS[messageName];
  if (schema === undefined) {
    throw new Error(`${messageName} isn't one of the six Connect JSON messages`);
  }
  return schema;
}

describe("Connect JSON cases agree with protobuf-es", () => {
  const cases = readConnectJsonCases();

  test.each(cases.encode)("$message from $binary is written as protobuf-es writes it", (connectCase) => {
    const schema = connectJsonSchema(connectCase.message);
    const bytes = new Uint8Array(readFileSync(new URL(`${CONNECT_JSON_FOLDER}/${connectCase.binary}`, GOLDEN_ROOT)));
    const message = fromBinary(schema, bytes, BINARY_OPTIONS);
    expect(toJsonString(schema, message, JSON_OPTIONS)).toBe(connectCase.json);
    expect(toBinary(schema, fromJsonString(schema, connectCase.json ?? ""))).toEqual(bytes);
  });

  test.each(cases.decode)("$message accepts $input", (connectCase) => {
    const schema = connectJsonSchema(connectCase.message);
    const message = fromJsonString(schema, connectCase.input ?? "");
    expect(toJsonString(schema, message, JSON_OPTIONS)).toBe(connectCase.json);
  });

  test.each(cases.refuse)("$message refuses $input", (connectCase) => {
    const schema = connectJsonSchema(connectCase.message);
    expect(() => fromJsonString(schema, connectCase.input ?? "")).toThrow();
  });

  test("every .binpb in the cases folder is an encode case", () => {
    /** @type {string[]} */
    const fileNames = readdirSync(new URL(`${CONNECT_JSON_FOLDER}/`, GOLDEN_ROOT));
    const binaryNames = fileNames.filter((name) => name.endsWith(BINARY_SUFFIX)).sort();
    // A case whose binary starts with "../" reuses a package golden; every other one lives in this folder.
    const caseBinaries = cases.encode.map((connectCase) => connectCase.binary ?? "");
    const localBinaries = caseBinaries.filter((name) => !name.startsWith("../")).sort();
    expect(localBinaries).toEqual(binaryNames);
  });
});
