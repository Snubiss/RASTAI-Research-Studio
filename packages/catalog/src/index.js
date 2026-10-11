// @rastai/catalog: the error and term catalog for the browser (contracts 2.3 and 10; layout rust-control-plane.md
// 4.9). Everything under generated/ is written by `just codegen` from catalog/ and the first-party plugin catalogs,
// and never edited; this file only re-exports it, so callers import from "@rastai/catalog" and name the entry types
// in JSDoc as `@typedef {import("@rastai/catalog").CatalogEntry} CatalogEntry`.

export { CATALOG, TERMS } from "./generated/catalog.js";

/** @typedef {import("./generated/catalog.js").CatalogEntry} CatalogEntry */
/** @typedef {import("./generated/catalog.js").TermEntry} TermEntry */
