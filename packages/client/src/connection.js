// connectToDaemon and checkHealth: the Connect transport and the credential headers (layout frontend-and-plugins.md
// 2.1, 3.2; contracts 3.2, 12.4). The only file of the client that imports connect-es and protobuf-es (contracts
// 2.5): the other files call the generated service stubs on the client object with plain request objects.

import { createClient } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import { CommandService, EventService } from "@rastai/proto";

import {
  AUTHORIZATION_HEADER,
  BEARER_PREFIX,
  CLIENT_HEADER,
  CLIENT_NAME_AND_VERSION,
  CODE_DAEMON_UNREACHABLE,
  HEALTH_PATH,
  HEALTH_STATUS_OK,
  SESSION_KEY_HEADER,
} from "./constants.js";
import { clientDiagnostic } from "./diagnostics.js";
import { execute } from "./execute.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("@connectrpc/connect").Interceptor} Interceptor */
/** @typedef {import("@connectrpc/connect").Transport} Transport */
/** @typedef {import("./types.js").ConnectOptions} ConnectOptions */
/** @typedef {import("./types.js").Credential} Credential */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */

// Methods a cookie request may send without the session key (contracts 3.2): they only read.
const READ_ONLY_METHODS = ["GET", "HEAD"];
// Every Connect call the client makes is a POST (connect-es sends GET only with useHttpGet, which is off).
const CONNECT_METHOD = "POST";
const TRAILING_SLASHES = /\/+$/;

/**
 * Checks the daemon's health, then builds a client.
 * @param {ConnectOptions} options
 * @returns {Promise<DaemonClient | Diagnostic>}   RRS-U0001 when the daemon isn't reachable
 */
export async function connectToDaemon(options) {
  const baseUrl = options.baseUrl.replace(TRAILING_SLASHES, "");
  const health = await checkHealth(baseUrl);
  if (!health.ok) {
    return clientDiagnostic(CODE_DAEMON_UNREACHABLE, { address: baseUrl });
  }
  // The event stream is binary only in the prototype (contracts 4), so only commands follow useJsonWire.
  const commandTransport = createTransport(baseUrl, options.credential, !options.useJsonWire);
  const eventTransport = createTransport(baseUrl, options.credential, true);
  /** @type {DaemonClient} */
  const client = {
    baseUrl,
    daemonVersion: health.version,
    credential: options.credential,
    commands: createClient(CommandService, commandTransport),
    events: createClient(EventService, eventTransport),
    // Here, so a DaemonClient satisfies CommandExecutor (contracts 12.4).
    execute: (commandId, args, executeOptions) => execute(client, commandId, args, executeOptions),
  };
  return client;
}

/**
 * Asks GET /api/health, which needs no credential. Not ok when the daemon can't be reached or isn't started.
 * @param {string} baseUrl
 * @returns {Promise<{ ok: boolean, version: string }>}
 */
export async function checkHealth(baseUrl) {
  try {
    const response = await fetch(daemonUrl(baseUrl, HEALTH_PATH), {
      headers: { [CLIENT_HEADER]: CLIENT_NAME_AND_VERSION },
    });
    if (!response.ok) {
      return { ok: false, version: "" };
    }
    const health = await response.json();
    return { ok: health.status === HEALTH_STATUS_OK, version: String(health.version ?? "") };
  } catch {
    // fetch rejects when nothing listens; a body that isn't JSON isn't our daemon either.
    return { ok: false, version: "" };
  }
}

/**
 * A daemon URL: the base URL without its trailing slashes, then the path, so "http://h/" and "http://h" agree.
 * @param {string} baseUrl
 * @param {string} path   starts with "/"
 * @returns {string}
 */
export function daemonUrl(baseUrl, path) {
  return baseUrl.replace(TRAILING_SLASHES, "") + path;
}

/**
 * The headers every request to the daemon carries: who the client is, and the credential. A bearer credential
 * sends its token on every request. A cookie credential leaves the cookie to the browser, never sets
 * Authorization, and adds the session key to every request other than GET or HEAD (ADR-0034 decision 10).
 * @param {Credential} credential
 * @param {string} method   the HTTP method, upper case
 * @returns {Record<string, string>}
 */
export function requestHeaders(credential, method) {
  /** @type {Record<string, string>} */
  const headers = { [CLIENT_HEADER]: CLIENT_NAME_AND_VERSION };
  if (credential.kind === "bearer") {
    headers[AUTHORIZATION_HEADER] = BEARER_PREFIX + (credential.token ?? "");
  } else if (!READ_ONLY_METHODS.includes(method)) {
    headers[SESSION_KEY_HEADER] = credential.sessionKey ?? "";
  }
  return headers;
}

/**
 * A Connect transport to the daemon with the credential headers on every call.
 * @param {string} baseUrl
 * @param {Credential} credential
 * @param {boolean} useBinaryFormat   protobuf binary, or Connect JSON for devtools
 * @returns {Transport}
 */
function createTransport(baseUrl, credential, useBinaryFormat) {
  return createConnectTransport({
    baseUrl,
    useBinaryFormat,
    interceptors: [credentialInterceptor(credential)],
  });
}

/**
 * @param {Credential} credential
 * @returns {Interceptor}
 */
function credentialInterceptor(credential) {
  const headers = requestHeaders(credential, CONNECT_METHOD);
  return (next) => (request) => {
    for (const [name, value] of Object.entries(headers)) {
      request.header.set(name, value);
    }
    return next(request);
  };
}
