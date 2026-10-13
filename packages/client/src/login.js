// Sign-in: the login code from the URL fragment, its removal, and its exchange for the session (layout
// frontend-and-plugins.md 2.1, 2.6; contracts 3.3). Only the fragment is read: a browser never sends it to a server
// or puts it in a Referer header, so the code stays out of every log.

import {
  CLIENT_HEADER,
  CLIENT_NAME_AND_VERSION,
  CODE_DAEMON_UNREACHABLE,
  CODE_LOGIN_CODE_REJECTED,
  HOME_ROUTE,
  JSON_CONTENT_TYPE,
  LOGIN_CODE_PARAMETER,
  LOGIN_PATH,
} from "./constants.js";
import { daemonUrl } from "./connection.js";
import { clientDiagnostic } from "./diagnostics.js";

/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */

const FRAGMENT_START = "#";
const ROUTE_START = "/";
const QUERY_START = "?";
const PAIR_SEPARATOR = "&";
const KEY_VALUE_SEPARATOR = "=";
// The exchange sends no credential: the code is the credential, and the session key is what it returns.
const LOGIN_METHOD = "POST";

/**
 * A fragment split as the router splits it (2.6): `[/<section_id>/<segments>][?<query>]`. A fragment that doesn't
 * start with "/" is a bare query with no route.
 * @typedef {object} FragmentParts
 * @property {string} route   "" for a bare query
 * @property {string} query   the text after "?", or the whole bare query
 */

/**
 * The login code in the URL fragment, or null when there is none. The real query string is never read.
 * @param {{ hash: string }} location   window.location, or any object with its hash
 * @returns {string | null}
 */
export function readLoginCode(location) {
  const { query } = splitFragment(location.hash);
  const code = new URLSearchParams(query).get(LOGIN_CODE_PARAMETER);
  return code === null || code === "" ? null : code;
}

/**
 * Rewrites the address bar's fragment without the login code, with history.replaceState, so the code never stays
 * in the address bar or the history. A bare `#login=<code>` becomes `#/home`. Other keys keep their exact text.
 * Does nothing when the fragment has no code.
 */
export function removeLoginCodeFromUrl() {
  const hash = window.location.hash;
  const { route, query } = splitFragment(hash);
  const pairs = query === "" ? [] : query.split(PAIR_SEPARATOR);
  const keptPairs = pairs.filter((pair) => !isLoginPair(pair));
  if (keptPairs.length === pairs.length) {
    return;
  }
  const keptQuery = keptPairs.join(PAIR_SEPARATOR);
  const newHash =
    FRAGMENT_START + (route === "" ? HOME_ROUTE : route) + (keptQuery === "" ? "" : QUERY_START + keptQuery);
  const newUrl = window.location.pathname + window.location.search + newHash;
  window.history.replaceState(window.history.state, "", newUrl);
}

/**
 * Redeems the code for the session cookie (set by the browser) and the session key (ADR-0034 decision 10), the
 * response body's session_key field: 64 lowercase hex characters, sent back unchanged (contracts 3.3).
 * @param {string} baseUrl
 * @param {string} code
 * @returns {Promise<{ sessionKey: string | null, diagnostic: Diagnostic | null }>}   RRS-U0002 when the code is
 *   rejected or expired, RRS-U0001 when the daemon can't be reached; the caller keeps sessionKey in memory only
 *   and passes it in the cookie credential
 */
export async function exchangeLoginCode(baseUrl, code) {
  let response;
  try {
    response = await fetch(daemonUrl(baseUrl, LOGIN_PATH), {
      method: LOGIN_METHOD,
      headers: { [CLIENT_HEADER]: CLIENT_NAME_AND_VERSION, "Content-Type": JSON_CONTENT_TYPE },
      body: JSON.stringify({ code }),
    });
  } catch {
    return { sessionKey: null, diagnostic: clientDiagnostic(CODE_DAEMON_UNREACHABLE, { address: baseUrl }) };
  }
  const sessionKey = response.ok ? await readSessionKey(response) : null;
  if (sessionKey === null) {
    return { sessionKey: null, diagnostic: clientDiagnostic(CODE_LOGIN_CODE_REJECTED) };
  }
  return { sessionKey, diagnostic: null };
}

/**
 * @param {Response} response
 * @returns {Promise<string | null>}   null when the body has no session_key text
 */
async function readSessionKey(response) {
  try {
    const body = await response.json();
    return typeof body.session_key === "string" && body.session_key !== "" ? body.session_key : null;
  } catch {
    return null;
  }
}

/**
 * @param {string} hash   location.hash: "" or "#..."
 * @returns {FragmentParts}
 */
function splitFragment(hash) {
  const fragment = hash.startsWith(FRAGMENT_START) ? hash.slice(FRAGMENT_START.length) : hash;
  if (!fragment.startsWith(ROUTE_START)) {
    return { route: "", query: fragment };
  }
  const queryStart = fragment.indexOf(QUERY_START);
  if (queryStart === -1) {
    return { route: fragment, query: "" };
  }
  return { route: fragment.slice(0, queryStart), query: fragment.slice(queryStart + QUERY_START.length) };
}

/**
 * True for a `login=<code>` pair; the key is compared decoded, as URLSearchParams reads it.
 * @param {string} pair
 * @returns {boolean}
 */
function isLoginPair(pair) {
  const key = pair.split(KEY_VALUE_SEPARATOR)[0];
  return new URLSearchParams(key + KEY_VALUE_SEPARATOR).has(LOGIN_CODE_PARAMETER);
}
