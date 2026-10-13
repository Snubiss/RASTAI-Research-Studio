// A fake daemon for the client's tests (layout frontend-and-plugins.md 3.5): connect-es's in-memory router serves
// CommandService with the real Connect protocol; EventService.Subscribe is answered by hand in the daemon's own
// stream format (control-plane layout 11.3: HTTP 200 at once, then envelopes), because the router holds a stream's
// headers back until its first message and the daemon doesn't; a few lines answer the plain HTTP paths (health,
// login, blobs). Tests install its `fetch` in place of the global one, so the client runs unchanged and
// every request it sends is recorded with its headers. Test support only; nothing in src/ outside tests imports it.

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { Code, ConnectError, createConnectRouter } from "@connectrpc/connect";
import { createFetchHandler } from "@connectrpc/connect/protocol";
import { BusEventSchema, CommandService, DiagnosticSchema, Severity, SubscribeRequestSchema } from "@rastai/proto";

/** @typedef {import("@rastai/proto").ExecuteRequest} ExecuteRequest */
/** @typedef {import("@rastai/proto").SubscribeRequest} SubscribeRequest */

const HEALTH_PATH = "/api/health";
const SUBSCRIBE_PATH = "/rastai.events.v1.EventService/Subscribe";
const STREAM_CONTENT_TYPE = "application/connect+proto";
// The stream envelope: one flags byte, then the message length as a big-endian u32 (the Connect protocol).
const ENVELOPE_HEADER_BYTES = 5;
const MESSAGE_FLAGS = 0;
const END_OF_STREAM_FLAGS = 2;
const LOGIN_PATH = "/api/login";
const BLOBS_PREFIX = "/api/blobs/";
const JSON_HEADERS = { "Content-Type": "application/json" };
const DAEMON_VERSION = "0.1.0";
// The daemon's gate answers a refused credential on a plain path with 401 and the Diagnostic as JSON.
const UNAUTHORIZED = 401;
const NOT_FOUND = 404;
const SESSION_REFUSED_CODE = "RRS-C1002";
// What a connection does after its events. "open" waits until the client hangs up; "drop" breaks the connection
// without a word; any other text is a catalog code the daemon ends the stream with.
const END_OPEN = "open";
const END_DROP = "drop";
// The hash every upload to the fake daemon is stored under.
export const UPLOADED_BLOB_HASH = "blake3:" + "ab".repeat(32);

/**
 * One Subscribe connection the fake answers: these events in order, then `end`.
 * @typedef {object} FakeConnection
 * @property {object[]} events   BusEvent fields in protobuf-es's camelCase
 * @property {string} end        "open", "drop" or a catalog code such as "RRS-C0401"
 */

/**
 * @typedef {object} RecordedRequest
 * @property {string} method
 * @property {string} path
 * @property {Headers} headers
 */

/**
 * @typedef {object} FakeDaemon
 * @property {string} baseUrl
 * @property {boolean} reachable              false: every request fails as when nothing listens
 * @property {string | null} refusal          a catalog code the gate refuses every request with (but health)
 * @property {(request: ExecuteRequest) => object} answerExecute   ExecuteResponse fields for each Execute call
 * @property {object[]} commands              what ListCommands answers: CommandDescriptor fields
 * @property {FakeConnection[]} connections   the Subscribe connections still to answer, first one first
 * @property {Record<string, string>} loginCodes   login code -> the session key it redeems for
 * @property {Record<string, Uint8Array>} blobs    blob hash -> its bytes
 * @property {RecordedRequest[]} requests
 * @property {ExecuteRequest[]} executeRequests
 * @property {SubscribeRequest[]} subscribeRequests
 * @property {(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>} fetch
 */

/**
 * A fake daemon at http://127.0.0.1:47820 that answers every Execute with ok and no result.
 * @returns {FakeDaemon}
 */
export function createFakeDaemon() {
  /** @type {FakeDaemon} */
  const daemon = {
    baseUrl: "http://127.0.0.1:47820",
    reachable: true,
    refusal: null,
    answerExecute: () => ({ ok: true }),
    commands: [],
    connections: [],
    loginCodes: {},
    blobs: {},
    requests: [],
    executeRequests: [],
    subscribeRequests: [],
    fetch: (input, init) => answerFetch(daemon, connectHandlers, new Request(input, init)),
  };
  const connectHandlers = createConnectHandlers(daemon);
  return daemon;
}

/**
 * The Connect error the daemon sends for a refusal: its one detail is the Diagnostic (control-plane layout 11.3).
 * @param {string} code
 * @returns {ConnectError}
 */
export function refusalError(code) {
  const diagnostic = create(DiagnosticSchema, { code, severity: Severity.ERROR, message: code });
  const connectCode = code === SESSION_REFUSED_CODE ? Code.Unauthenticated : Code.FailedPrecondition;
  return new ConnectError(code, connectCode, undefined, [{ desc: DiagnosticSchema, value: diagnostic }]);
}

/**
 * The router's handlers by request path, each turned into a function from a fetch Request to a Response.
 * @param {FakeDaemon} daemon
 * @returns {Map<string, (request: Request) => Promise<Response>>}
 */
function createConnectHandlers(daemon) {
  const router = createConnectRouter();
  router.service(CommandService, {
    execute: (request) => answerExecute(daemon, request),
    listCommands: () => ({ commands: daemon.commands }),
  });
  const handlers = new Map();
  for (const handler of router.handlers) {
    handlers.set(handler.requestPath, createFetchHandler(handler));
  }
  return handlers;
}

/**
 * @param {FakeDaemon} daemon
 * @param {Map<string, (request: Request) => Promise<Response>>} connectHandlers
 * @param {Request} request
 * @returns {Promise<Response>}
 */
async function answerFetch(daemon, connectHandlers, request) {
  const path = new URL(request.url).pathname;
  daemon.requests.push({ method: request.method, path, headers: request.headers });
  if (!daemon.reachable) {
    // What fetch does when nothing listens on the port.
    throw new TypeError("fetch failed");
  }
  if (path === HEALTH_PATH) {
    return Response.json({ status: "ok", version: DAEMON_VERSION, apis: [], plugin_api: "1.0", node_protocol: 1 });
  }
  if (path === SUBSCRIBE_PATH) {
    return answerSubscribe(daemon, request);
  }
  const connectHandler = connectHandlers.get(path);
  if (connectHandler !== undefined) {
    return connectHandler(request);
  }
  if (daemon.refusal !== null) {
    return Response.json(
      { code: daemon.refusal, severity: "error", message: daemon.refusal },
      { status: UNAUTHORIZED },
    );
  }
  if (path === LOGIN_PATH) {
    return answerLogin(daemon, request);
  }
  if (path.startsWith(BLOBS_PREFIX) && request.method === "GET") {
    return answerBlobRead(daemon, decodeURIComponent(path.slice(BLOBS_PREFIX.length)));
  }
  return new Response(null, { status: NOT_FOUND, headers: JSON_HEADERS });
}

/**
 * @param {FakeDaemon} daemon
 * @param {ExecuteRequest} request
 * @returns {object}
 */
function answerExecute(daemon, request) {
  if (daemon.refusal !== null) {
    throw refusalError(daemon.refusal);
  }
  daemon.executeRequests.push(request);
  return daemon.answerExecute(request);
}

/**
 * Plays the next scripted connection: HTTP 200 at once, its events, then its end. With no script left the stream
 * stays open. A refusal, like the daemon's after its gate, is the end-of-stream envelope alone.
 * @param {FakeDaemon} daemon
 * @param {Request} request
 * @returns {Promise<Response>}
 */
async function answerSubscribe(daemon, request) {
  const headers = { "Content-Type": STREAM_CONTENT_TYPE };
  if (daemon.refusal !== null) {
    return new Response(endOfStreamEnvelope(daemon.refusal).slice(), { headers });
  }
  const body = new Uint8Array(await request.arrayBuffer());
  daemon.subscribeRequests.push(fromBinary(SubscribeRequestSchema, body.subarray(ENVELOPE_HEADER_BYTES)));
  const connection = daemon.connections.shift() ?? { events: [], end: END_OPEN };
  return new Response(connectionBody(connection), { headers });
}

/**
 * The body of one scripted connection, one envelope per read, so a drop comes after the events were read (a
 * stream that errors throws away what is still queued).
 * @param {FakeConnection} connection
 * @returns {ReadableStream<Uint8Array>}
 */
function connectionBody(connection) {
  const envelopes = connection.events.map((event) =>
    envelope(MESSAGE_FLAGS, toBinary(BusEventSchema, create(BusEventSchema, event))),
  );
  return new ReadableStream({
    pull(controller) {
      const next = envelopes.shift();
      if (next !== undefined) {
        controller.enqueue(next);
      } else if (connection.end === END_DROP) {
        controller.error(new TypeError("connection reset"));
      } else if (connection.end !== END_OPEN) {
        controller.enqueue(endOfStreamEnvelope(connection.end));
        controller.close();
      } else {
        // END_OPEN: nothing more until the client hangs up, which cancels the stream.
        return new Promise(() => {});
      }
    },
  });
}

/**
 * The last envelope of a stream that ends with a refusal: `{"error": <the Connect error>}`, whose one detail is the
 * Diagnostic as base64 protobuf, as the daemon writes it.
 * @param {string} code
 * @returns {Uint8Array}
 */
function endOfStreamEnvelope(code) {
  const diagnostic = create(DiagnosticSchema, { code, severity: Severity.ERROR, message: code });
  const detail = { type: DiagnosticSchema.typeName, value: toBase64(toBinary(DiagnosticSchema, diagnostic)) };
  const connectCode = code === SESSION_REFUSED_CODE ? "unauthenticated" : "failed_precondition";
  const error = { code: connectCode, message: code, details: [detail] };
  return envelope(END_OF_STREAM_FLAGS, new TextEncoder().encode(JSON.stringify({ error })));
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
 * @param {Uint8Array} bytes
 * @returns {string}
 */
function toBase64(bytes) {
  return btoa(String.fromCharCode(...bytes));
}

/**
 * @param {FakeDaemon} daemon
 * @param {Request} request
 * @returns {Promise<Response>}
 */
async function answerLogin(daemon, request) {
  const body = await request.json();
  const sessionKey = daemon.loginCodes[body.code];
  if (sessionKey === undefined) {
    return Response.json({ code: "RRS-C1007", severity: "error", message: "used" }, { status: UNAUTHORIZED });
  }
  delete daemon.loginCodes[body.code];
  return Response.json({ session_key: sessionKey });
}

/**
 * @param {FakeDaemon} daemon
 * @param {string} blobHash
 * @returns {Response}
 */
function answerBlobRead(daemon, blobHash) {
  const bytes = daemon.blobs[blobHash];
  if (bytes === undefined) {
    return Response.json({ code: "RRS-S1002", severity: "error", message: blobHash }, { status: NOT_FOUND });
  }
  // A copy, so the response owns its bytes.
  return new Response(bytes.slice());
}

/**
 * A stand-in for the browser's XMLHttpRequest (Node has none) that sends uploads to the fake daemon: it records
 * the request like fetch does, reports progress at half and at the end, and answers PUT /api/blobs with a BlobRef.
 * A class, because the client constructs it with `new`, as the browser API demands.
 * @param {FakeDaemon} daemon
 */
export function fakeXmlHttpRequestClass(daemon) {
  return class FakeXmlHttpRequest {
    constructor() {
      this.method = "";
      this.url = "";
      this.headers = new Headers();
      this.status = 0;
      this.responseText = "";
      /** @type {{ onprogress: ((progress: { loaded: number, total: number }) => void) | null }} */
      this.upload = { onprogress: null };
      /** @type {(() => void) | null} */
      this.onload = null;
      /** @type {(() => void) | null} */
      this.onerror = null;
    }

    /**
     * @param {string} method
     * @param {string} url
     */
    open(method, url) {
      this.method = method;
      this.url = url;
    }

    /**
     * @param {string} name
     * @param {string} value
     */
    setRequestHeader(name, value) {
      this.headers.set(name, value);
    }

    /** @param {Blob} body */
    send(body) {
      daemon.requests.push({ method: this.method, path: new URL(this.url).pathname, headers: this.headers });
      queueMicrotask(() => this.answer(body));
    }

    /** @param {Blob} body */
    answer(body) {
      if (!daemon.reachable) {
        this.onerror?.();
        return;
      }
      if (daemon.refusal !== null) {
        this.status = UNAUTHORIZED;
        this.responseText = JSON.stringify({ code: daemon.refusal, severity: "error", message: daemon.refusal });
        this.onload?.();
        return;
      }
      this.upload.onprogress?.({ loaded: Math.floor(body.size / 2), total: body.size });
      this.upload.onprogress?.({ loaded: body.size, total: body.size });
      this.status = 200;
      this.responseText = JSON.stringify({
        hash: UPLOADED_BLOB_HASH,
        size_bytes: String(body.size),
        media_type: body.type,
      });
      this.onload?.();
    }
  };
}
