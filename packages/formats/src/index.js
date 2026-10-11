// @rastai/formats: the document and command types for the browser (contracts 2.3 and 5.4; layout
// rust-control-plane.md 4.9). Everything under generated/ is written by `just codegen` from the type descriptions in
// crates/rastai-formats and never edited; this file only re-exports it, so callers name a type in JSDoc as
// `@typedef {import("@rastai/formats").SharedHeader} SharedHeader`. The rastai-formats WASM validator joins here in M4.

export * from "./generated/index.js";
