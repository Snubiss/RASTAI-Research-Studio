// Navigation through the address bar's fragment (layout frontend-and-plugins.md 2.6). The fragment is the one source
// of the current route: navigate writes it, and watchRoute reads it on every hashchange into the route store.

import { formatRoute, parseRoute } from "./route.js";

/** @typedef {import("./route.js").Route} Route */
/** @typedef {import("@rastai/plugin").Store} Store */

/**
 * Moves to a route. A route equal to the current one writes nothing, so no history entry is added for it.
 * @param {Route} route
 */
export function navigateTo(route) {
  const fragment = formatRoute(route);
  if (window.location.hash !== fragment) {
    window.location.hash = fragment;
  }
}

/**
 * Moves to a fragment the caller already has, such as one a plugin passed to host.navigate.
 * @param {string} fragment
 */
export function navigateToFragment(fragment) {
  navigateTo(parseRoute(fragment));
}

/**
 * Keeps the route store equal to the address bar: it is set now, and again on every hashchange.
 * @param {Store} routeStore
 * @returns {() => void} stops watching
 */
export function watchRoute(routeStore) {
  const update = () => routeStore.set(parseRoute(window.location.hash));
  update();
  window.addEventListener("hashchange", update);
  return () => window.removeEventListener("hashchange", update);
}
