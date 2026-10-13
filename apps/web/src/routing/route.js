// The route and the fragment grammar of the web app (layout frontend-and-plugins.md 2.6): `#/<section_id>/<segments>?<query>`.
// Pure functions only. The login code is read and removed before the router runs (@rastai/client), so `login` is not
// a route key here. A fragment with no leading "/" is a bare query, and its route is Home.

import { HOME_SECTION_ID } from "../constants.js";

/**
 * A parsed route: which section, its path segments and its query.
 * @typedef {object} Route
 * @property {string} sectionId          e.g. "inference"; "home" for an empty or bare fragment
 * @property {string[]} path             the segments after the section, decoded
 * @property {Object<string, string>} query
 */

const FRAGMENT_START = "#";
const ROUTE_START = "/";
const QUERY_START = "?";
const SEGMENT_SEPARATOR = "/";
const PAIR_SEPARATOR = "&";
// A login code is never a route key, so the router never sees one in a query (layout 2.6).
const RESERVED_QUERY_KEY = "login";

/**
 * @param {string} fragment   the text after "#", or with it: window.location.hash either way
 * @returns {Route}
 */
export function parseRoute(fragment) {
  const text = fragment.startsWith(FRAGMENT_START) ? fragment.slice(FRAGMENT_START.length) : fragment;
  const queryStart = text.indexOf(QUERY_START);
  const routePart = queryStart === -1 ? text : text.slice(0, queryStart);
  const queryPart = queryStart === -1 ? "" : text.slice(queryStart + QUERY_START.length);
  const segments = routeSegments(routePart);
  return {
    sectionId: segments.length === 0 ? HOME_SECTION_ID : segments[0],
    path: segments.slice(1),
    query: parseQuery(queryPart),
  };
}

/**
 * The fragment for a route, with its query percent-encoded: parseRoute reads it back to the same route.
 * @param {Route} route
 * @returns {string}   with the leading "#"
 */
export function formatRoute(route) {
  const segments = [route.sectionId, ...route.path].map((segment) => encodeURIComponent(segment));
  const query = Object.entries(route.query)
    .filter(([key]) => key !== RESERVED_QUERY_KEY)
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`)
    .join(PAIR_SEPARATOR);
  const queryText = query === "" ? "" : QUERY_START + query;
  return FRAGMENT_START + ROUTE_START + segments.join(SEGMENT_SEPARATOR) + queryText;
}

/**
 * The route a section's own page opens with, from its id alone.
 * @param {string} sectionId
 * @returns {Route}
 */
export function sectionRoute(sectionId) {
  return { sectionId, path: [], query: {} };
}

/**
 * The fragment of a section's own page, e.g. "#/home".
 * @param {string} sectionId
 * @returns {string}
 */
export function sectionFragment(sectionId) {
  return formatRoute(sectionRoute(sectionId));
}

/**
 * @param {string} routePart   the text before "?", without its leading "#"
 * @returns {string[]}   the decoded, non-empty segments
 */
function routeSegments(routePart) {
  if (!routePart.startsWith(ROUTE_START)) {
    return [];
  }
  return routePart
    .split(SEGMENT_SEPARATOR)
    .filter((segment) => segment !== "")
    .map((segment) => safeDecode(segment));
}

/**
 * @param {string} queryPart
 * @returns {Object<string, string>}
 */
function parseQuery(queryPart) {
  /** @type {Object<string, string>} */
  const query = {};
  for (const [key, value] of new URLSearchParams(queryPart)) {
    if (key !== RESERVED_QUERY_KEY) {
      query[key] = value;
    }
  }
  return query;
}

/**
 * A segment that isn't valid percent-encoding is kept as it was written, rather than failing the whole route.
 * @param {string} segment
 * @returns {string}
 */
function safeDecode(segment) {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}
