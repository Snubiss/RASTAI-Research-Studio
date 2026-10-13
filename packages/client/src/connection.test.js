// connectToDaemon and the credential headers (layout frontend-and-plugins.md 3.5): which header each credential
// sends on commands, the event stream and blobs, the JSON wire switch, and an unreachable daemon.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { readBlobBytes, uploadBlob } from "./blobs.js";
import { connectToDaemon } from "./connection.js";
import { openEventStream } from "./event-stream.js";
import { createFakeDaemon, fakeXmlHttpRequestClass } from "./testing/fake-daemon.js";

/** @typedef {import("./testing/fake-daemon.js").FakeDaemon} FakeDaemon */
/** @typedef {import("./testing/fake-daemon.js").RecordedRequest} RecordedRequest */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */
/** @typedef {import("./types.js").Credential} Credential */

const TOKEN = "tok_secret_0123";
const SESSION_KEY = "5d".repeat(32);
const EXECUTE_PATH = "/rastai.commands.v1.CommandService/Execute";
const SUBSCRIBE_PATH = "/rastai.events.v1.EventService/Subscribe";
const BLOB_HASH = "blake3:" + "cd".repeat(32);
const BLOB_READ_PATH = `/api/blobs/${encodeURIComponent(BLOB_HASH)}`;
const BLOB_UPLOAD_PATH = "/api/blobs";
const HEALTH_PATH = "/api/health";

/** @type {FakeDaemon} */
let daemon;

beforeEach(() => {
  daemon = createFakeDaemon();
  daemon.blobs[BLOB_HASH] = new Uint8Array([1, 2, 3]);
  vi.stubGlobal("fetch", daemon.fetch);
  vi.stubGlobal("XMLHttpRequest", fakeXmlHttpRequestClass(daemon));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/**
 * @param {Credential} credential
 * @param {boolean} [useJsonWire]
 * @returns {Promise<DaemonClient>}
 */
async function connect(credential, useJsonWire) {
  const client = await connectToDaemon({ baseUrl: daemon.baseUrl, credential, useJsonWire });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  return client;
}

/**
 * One execute, one event stream that opens, one blob read and one upload: every kind of request the client makes.
 * @param {DaemonClient} client
 */
async function makeEveryKindOfRequest(client) {
  await client.execute("system.status", {});
  const stream = openEventStream(client, [], 0n, { onEvent: () => {}, onStatus: () => {}, onResync: () => {} });
  await vi.waitFor(() => expect(daemon.subscribeRequests).toHaveLength(1));
  stream.stop();
  await readBlobBytes(client, BLOB_HASH);
  await uploadBlob(client, new Blob(["hello"], { type: "text/plain" }), () => {});
}

/**
 * @param {string} method
 * @param {string} path
 * @returns {RecordedRequest}
 */
function requestTo(method, path) {
  const found = daemon.requests.find((request) => request.method === method && request.path === path);
  if (found === undefined) {
    throw new Error(`no ${method} ${path} among ${daemon.requests.map((request) => request.path).join(", ")}`);
  }
  return found;
}

describe("a bearer credential", () => {
  test("sends Authorization: Bearer <token> on commands, the stream and blob reads", async () => {
    const client = await connect({ kind: "bearer", token: TOKEN });
    await makeEveryKindOfRequest(client);
    for (const [method, path] of [
      ["POST", EXECUTE_PATH],
      ["POST", SUBSCRIBE_PATH],
      ["GET", BLOB_READ_PATH],
      ["PUT", BLOB_UPLOAD_PATH],
    ]) {
      expect(requestTo(method, path).headers.get("Authorization"), `${method} ${path}`).toBe(`Bearer ${TOKEN}`);
    }
  });

  test("never sends a session key", async () => {
    const client = await connect({ kind: "bearer", token: TOKEN });
    await makeEveryKindOfRequest(client);
    for (const request of daemon.requests) {
      expect(request.headers.has("Rastai-Session-Key"), `${request.method} ${request.path}`).toBe(false);
    }
  });
});

describe("a cookie credential", () => {
  test("never sets Authorization", async () => {
    const client = await connect({ kind: "cookie", sessionKey: SESSION_KEY });
    await makeEveryKindOfRequest(client);
    expect(daemon.requests.length).toBeGreaterThanOrEqual(5);
    for (const request of daemon.requests) {
      expect(request.headers.has("Authorization"), `${request.method} ${request.path}`).toBe(false);
    }
  });

  test("sends Rastai-Session-Key on Execute, Subscribe and blob uploads", async () => {
    const client = await connect({ kind: "cookie", sessionKey: SESSION_KEY });
    await makeEveryKindOfRequest(client);
    for (const [method, path] of [
      ["POST", EXECUTE_PATH],
      ["POST", SUBSCRIBE_PATH],
      ["PUT", BLOB_UPLOAD_PATH],
    ]) {
      expect(requestTo(method, path).headers.get("Rastai-Session-Key"), `${method} ${path}`).toBe(SESSION_KEY);
    }
  });

  test("sends no session key on blob reads or the health check", async () => {
    const client = await connect({ kind: "cookie", sessionKey: SESSION_KEY });
    await makeEveryKindOfRequest(client);
    expect(requestTo("GET", BLOB_READ_PATH).headers.has("Rastai-Session-Key")).toBe(false);
    expect(requestTo("GET", HEALTH_PATH).headers.has("Rastai-Session-Key")).toBe(false);
  });
});

test("every request names the client in Rastai-Client", async () => {
  const client = await connect({ kind: "cookie", sessionKey: SESSION_KEY });
  await makeEveryKindOfRequest(client);
  for (const request of daemon.requests) {
    expect(request.headers.get("Rastai-Client"), `${request.method} ${request.path}`).toBe("rastai-client/0.1.0");
  }
});

test("useJsonWire sends commands as Connect JSON and keeps the stream binary", async () => {
  const jsonClient = await connect({ kind: "cookie", sessionKey: SESSION_KEY }, true);
  await makeEveryKindOfRequest(jsonClient);
  expect(requestTo("POST", EXECUTE_PATH).headers.get("Content-Type")).toBe("application/json");
  expect(requestTo("POST", SUBSCRIBE_PATH).headers.get("Content-Type")).toBe("application/connect+proto");
});

test("commands are protobuf binary by default", async () => {
  const client = await connect({ kind: "cookie", sessionKey: SESSION_KEY });
  await client.execute("system.status", {});
  expect(requestTo("POST", EXECUTE_PATH).headers.get("Content-Type")).toBe("application/proto");
});

test("the client records the daemon's version and a base URL without trailing slashes", async () => {
  const client = await connectToDaemon({
    baseUrl: `${daemon.baseUrl}/`,
    credential: { kind: "bearer", token: TOKEN },
  });
  expect("execute" in client && client.daemonVersion).toBe("0.1.0");
  expect("execute" in client && client.baseUrl).toBe(daemon.baseUrl);
  expect(requestTo("GET", HEALTH_PATH).path).toBe(HEALTH_PATH);
});

test("an unreachable daemon gives RRS-U0001 naming its address", async () => {
  daemon.reachable = false;
  const result = await connectToDaemon({ baseUrl: daemon.baseUrl, credential: { kind: "bearer", token: TOKEN } });
  expect("code" in result && result.code).toBe("RRS-U0001");
  expect("code" in result && result.message).toBe(`The daemon at ${daemon.baseUrl} is not reachable.`);
});
