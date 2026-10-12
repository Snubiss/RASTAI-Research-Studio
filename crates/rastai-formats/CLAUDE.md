# rastai-formats

The one owner of every document type and of the byte formats Rastai reads and writes. Its design is
`docs/architecture/layout/rust-control-plane.md` section 4; its names are in `docs/architecture/contracts.md` §2.1
and §15.1.

## Local rules

- **Bytes in, typed data out.** Never opens a path, never touches the network, SQLite or the environment. File
  access is only through a `FileSource` or `FileSink` the caller opened.
- **No tokio, and no rastai crate except `rastai-diag`.** The crate is the base of `rastai-core` and of the WASM and
  Python builds, so it can't pull in a runtime or anything above it.
- **WASM-safe.** It must build for `wasm32-unknown-unknown` (`just wasm`). Code that needs the operating system
  sits behind `#[cfg(...)]` with a portable fallback.
- **Untrusted input is checked before allocating** (`ImportLimits`, ADR-0032 decision 8), never after.
- **The JSON reader is in-house** (`src/json/reader.rs`); serde_json only holds the values and writes the pretty
  form. Every in-house parser of untrusted input has a fuzz target in `fuzz/` (ADR-0037).
- **Content hashes are computed only here** (canonical JSON, then BLAKE3). JavaScript and Python call this code
  through WASM and the wheel and never hash documents themselves.
- **One wrapper file per external crate:** `blake3` only in `src/hashing.rs`; `miniz_oxide` only in
  `src/zip/inflate.rs` (M2).
- **One error enum**, `FormatsError` in `src/error.rs`, each variant with a catalog code from contracts §10.3.
- **A document type is written in four places, all in its own contract PR:** the struct (with
  `#[serde(flatten)] pub header: SharedHeader`) and its `describe_<file>_types()` (the document's fields start with
  `header_fields()`; its parts follow) in `src/documents/<type>.rs`; its row in `DOCUMENT_TYPES`, its `Document`
  variant and its arms in `document_from_value`, `document_header` and `document_header_mut`
  (`src/documents/mod.rs`) and in `check_meaning` (`src/validate/meaning/mod.rs`); its descriptions in
  `describe_all_types()` (`src/describe/registry.rs`); its fixtures in `fixtures/documents/<type>/` with a
  `tests/describe_matches_struct.rs` case for it and every part; and each valid fixture's content hash in
  `fixtures/documents/content_hashes.json`, which Rust and the wheel are both checked against.
- **`src/lib.rs` and `src/json/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the
  bottom of the file they test.

## Fuzzing

`fuzz/` is a cargo-fuzz crate with its own workspace, built on the pinned nightly:
`just fuzz rastai-formats json_reader 60` runs the reader target for a minute, and `read_document` the same way.
Nightly CI runs each target for 10 minutes (contracts §12.5).
