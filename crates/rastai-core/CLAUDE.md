# rastai-core

The shared vocabulary of the Rust processes that isn't a document. Its design is
`docs/architecture/layout/rust-control-plane.md` section 5; its names are in `docs/architecture/contracts.md` §2.1
and §15.1.

## Local rules

- **Synchronous, and no I/O.** No tokio, axum, tonic, rusqlite or network crate. No files, no sockets, no SQLite.
  The only reads from the operating system are environment variables (`read_path_environment`, and settings from
  M1-21), the owner of `/proc/self` (the user id, in `read_path_environment`) and `/dev/urandom` (`random.rs`).
- **Fuzz targets** for the parsers of untrusted text live in `fuzz/` (its own nightly workspace, contracts §12.5):
  `base32_decode` and `rfc3339_parse`. A new parser here adds its target in the same PR (ADR-0037).
- **The only place data paths are built** (contracts §8.1, ADR-0014). No other crate reads `RASTAI_HOME`,
  `HOME`, `XDG_*`, `HF_HUB_CACHE` or `HF_HOME`; inside this crate only `src/paths.rs` does.
- **No twins of generated types.** State enums, `Diagnostic` and `CommandDescriptor` on the wire are
  `rastai-proto`'s; this crate adds names, transition tables and conversions. The two allowed in-process twins,
  rastai-diag's `Diagnostic` and `CommandRow`, are pinned field by field by `tests/wire_round_trip.rs`.
- **`COMMANDS` holds core commands only** (M1-21). Plugin commands come from manifests at runtime.
- **One error enum**, `CoreError` in `src/error.rs`, each variant with a catalog code from contracts §10.3.
- **`src/lib.rs` and `src/commands/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the
  bottom of the file they test; the wire test is in `tests/`.
