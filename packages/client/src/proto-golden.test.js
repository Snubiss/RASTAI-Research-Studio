// The JS leg of the golden test on the client's own wire path (layout frontend-and-plugins.md 3.5). Every message
// in fixtures/proto-golden/ already decodes and re-encodes byte-identical in packages/proto/src/golden.test.js;
// here the golden bytes go through the client itself, as a daemon would send them, and what the client hands its
// caller must encode back to exactly those bytes. The requests the client writes must equal their goldens.

// @ts-expect-error -- no Node.js type definitions in this workspace (@types/node is not in spec §27)
import { readFileSync } from "node:fs";

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  BusEventSchema,
  ExecuteResponseSchema,
  ListCommandsResponseSchema,
  SubscribeRequestSchema,
} from "@rastai/proto";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { connectToDaemon } from "./connection.js";
import { openEventStream } from "./event-stream.js";
import { listCommands } from "./execute.js";

/** @typedef {import("@rastai/proto").BusEvent} BusEvent */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */

const GOLDEN_ROOT = new URL("../../../fixtures/proto-golden/", import.meta.url);
const BASE_URL = "http://127.0.0.1:47820";
const HEALTH_PATH = "/api/health";
const EXECUTE_PATH = "/rastai.commands.v1.CommandService/Execute";
const LIST_COMMANDS_PATH = "/rastai.commands.v1.CommandService/ListCommands";
const SUBSCRIBE_PATH = "/rastai.events.v1.EventService/Subscribe";
const ENVELOPE_HEADER_BYTES = 5;
const MESSAGE_FLAGS = 0;
const END_OF_STREAM_FLAGS = 2;

/**
 * The golden bytes the fake answers each Connect path with, and the request bodies the client sent.
 * @type {{ answers: Record<string, Uint8Array>, bodies: Record<string, Uint8Array> }}
 */
let wire;

/**
 * @param {string} folder
 * @param {string} messageName
 * @returns {Uint8Array}
 */
function readGoldenBytes(folder, messageName) {
  return new Uint8Array(readFileSync(new URL(`${folder}/${messageName}.binpb`, GOLDEN_ROOT)));
}

/**
 * @param {string} folder
 * @param {string} messageName
 */
function readGoldenJson(folder, messageName) {
  return JSON.parse(readFileSync(new URL(`${folder}/${messageName}.json`, GOLDEN_ROOT), "utf8"));
}

/**
 * @param {number} flags
 * @param {Uint8Array} message
 * @returns {Uint8Array}
 */
function envelope(flags, message) {
  const bytes = new Uint8Array(ENVELOPE_HEADER_BYTES + message.length);
  bytes[0] = flags;
  new DataView(bytes.buffer).setUint32(1, message.length);
  bytes.set(message, ENVELOPE_HEADER_BYTES);
  return bytes;
}

/**
 * A daemon that answers each Connect path with the raw golden bytes set for it, unary or in one stream envelope.
 * @param {RequestInfo | URL} input
 * @param {RequestInit} [init]
 * @returns {Promise<Response>}
 */
async function goldenFetch(input, init) {
  const request = new Request(input, init);
  const path = new URL(request.url).pathname;
  if (path === HEALTH_PATH) {
    return Response.json({ status: "ok", version: "0.1.0" });
  }
  wire.bodies[path] = new Uint8Array(await request.arrayBuffer());
  const answer = wire.answers[path];
  if (path === SUBSCRIBE_PATH) {
    const end = envelope(END_OF_STREAM_FLAGS, new TextEncoder().encode("{}"));
    const body = new Blob([envelope(MESSAGE_FLAGS, answer).slice(), end.slice()]);
    return new Response(body, { headers: { "Content-Type": "application/connect+proto" } });
  }
  return new Response(answer.slice(), { headers: { "Content-Type": "application/proto" } });
}

beforeEach(() => {
  wire = { answers: {}, bodies: {} };
  vi.stubGlobal("fetch", goldenFetch);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** @returns {Promise<DaemonClient>} */
async function connect() {
  const client = await connectToDaemon({ baseUrl: BASE_URL, credential: { kind: "bearer", token: "t" } });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  return client;
}

test("execute writes the ExecuteRequest golden byte for byte", async () => {
  const golden = readGoldenJson("commands", "ExecuteRequest");
  // The golden's fixed key in place of a fresh one.
  vi.stubGlobal("crypto", { randomUUID: () => golden.idempotency_key });
  wire.answers[EXECUTE_PATH] = readGoldenBytes("commands", "ExecuteResponse");
  const client = await connect();
  await client.execute(golden.command_id, JSON.parse(golden.arguments_json), {
    projectId: golden.project_id,
    dryRun: golden.dry_run,
    actingPluginId: golden.acting_plugin_id,
  });
  expect(wire.bodies[EXECUTE_PATH]).toEqual(readGoldenBytes("commands", "ExecuteRequest"));
});

test("the ExecuteResponse golden comes through execute with nothing lost", async () => {
  const goldenBytes = readGoldenBytes("commands", "ExecuteResponse");
  wire.answers[EXECUTE_PATH] = goldenBytes;
  const client = await connect();
  const outcome = await client.execute("model.load", {});
  const rebuilt = create(ExecuteResponseSchema, {
    ok: outcome.ok,
    resultJson: JSON.stringify(outcome.result),
    jobId: outcome.jobId ?? "",
    diagnostics: outcome.diagnostics,
    journalId: outcome.journalId ?? "",
  });
  expect(toBinary(ExecuteResponseSchema, rebuilt)).toEqual(goldenBytes);
});

test("the ListCommandsResponse golden comes through listCommands with nothing lost", async () => {
  const goldenBytes = readGoldenBytes("commands", "ListCommandsResponse");
  wire.answers[LIST_COMMANDS_PATH] = goldenBytes;
  const client = await connect();
  const commands = await listCommands(client);
  expect(toBinary(ListCommandsResponseSchema, create(ListCommandsResponseSchema, { commands }))).toEqual(goldenBytes);
});

test("the BusEvent golden comes through the event stream byte-identical", async () => {
  const goldenBytes = readGoldenBytes("events", "BusEvent");
  wire.answers[SUBSCRIBE_PATH] = goldenBytes;
  const client = await connect();
  /** @type {BusEvent[]} */
  const received = [];
  const stream = openEventStream(client, [], 0n, {
    onEvent: (event) => received.push(event),
    onStatus: () => {},
    onResync: () => {},
  });
  await vi.waitFor(() => expect(received).toHaveLength(1));
  stream.stop();
  expect(toBinary(BusEventSchema, received[0])).toEqual(goldenBytes);
});

test("the stream sends its SubscribeRequest in one envelope, as the daemon reads it", async () => {
  const golden = readGoldenJson("events", "SubscribeRequest");
  wire.answers[SUBSCRIBE_PATH] = readGoldenBytes("events", "BusEvent");
  const client = await connect();
  const stream = openEventStream(client, golden.event_types, BigInt(golden.after_bus_sequence), {
    onEvent: () => stream.stop(),
    onStatus: () => {},
    onResync: () => {},
  });
  await vi.waitFor(() => expect(wire.bodies[SUBSCRIBE_PATH]).toBeDefined());
  const sent = wire.bodies[SUBSCRIBE_PATH];
  expect(sent[0]).toBe(MESSAGE_FLAGS);
  expect(new DataView(sent.buffer).getUint32(1)).toBe(sent.length - ENVELOPE_HEADER_BYTES);
  const request = fromBinary(SubscribeRequestSchema, sent.subarray(ENVELOPE_HEADER_BYTES));
  // openEventStream filters by event type only, so the golden's subject ids are the one field it leaves out.
  const expected = create(SubscribeRequestSchema, {
    afterBusSequence: BigInt(golden.after_bus_sequence),
    eventTypes: golden.event_types,
  });
  expect(toBinary(SubscribeRequestSchema, request)).toEqual(toBinary(SubscribeRequestSchema, expected));
});
