# ADR-0032: Document files are JSON with a shared header, parsed once, validated in four layers

**Status:** Accepted, 2026-10-10

*Note since acceptance (2026-10-10): the daemon-free CLI validator in decision 6 is the local command `rastai validate <file>` (control-plane layout §9.1, §9.7). `rastai file validate` is the generated verb for `file.validate`, which needs a running daemon. The decision is unchanged.*

## Context

- Rastai wants to be the default file format for shared architectures, harnesses, datasets, evals and replays (§2). People will diff, edit, review and share these files.
- Old files must always open. Files from a newer version must not be silently mangled.
- Imported files are untrusted: zip bombs, zip-slip paths, absurd sizes (§23).
- Bare `.rrs` is taken (Road Rash saves, R-Studio recovery files).
- Errors must be compiler-quality: all of them at once, each with a path and a fix (§23).

## Decision

1. **Every definition document is JSON**, with a `$schema` key so editors validate it. Containers (`.rrsreplay`, `.rrsplugin`, zipped `.rrsproj`) are Zip files with a JSON document at a fixed path.
2. **Extensions** are always typed: `.rrsmodel`, `.rrsharness`, `.rrsdata`, `.rrseval`, `.rrsreplay`, `.rrsplugin`, `.rrsproj`. Never bare `.rrs`.
3. **Shared header** on every document: `$schema`, `type`, `schema_version`, `created_with`, `id`, `content_hash`, `derived_from`, `authors`, `license`, `source_url`, `requires`.
   - `content_hash` is BLAKE3 over canonical JSON (keys sorted, no insignificant whitespace, UTF-8) with `content_hash` itself removed, written `blake3:<64 hex>`.
4. **Schema ids** are `<type>/<schema_version>` (for example `rastai.model/1`). `schema_version` is an integer per type.
5. **Migrations:** on load, `schema_version` drives a chain of plain functions (`migrate_<type>_v1_to_v2`, ...). Old files always open. Files from a newer version open **read-only** with an `RRS-E4xxx` notice. `file.migrate` writes an upgraded copy and never overwrites in place.
6. **Parse, don't validate.** `rastai-formats` turns bytes into typed Rust structs at the boundary, and internal code never re-checks. The same crate runs in the daemon, the CLI (`rastai file validate`), the browser (WASM) and Python (PyO3).
7. **Four layers, one pass, all diagnostics:**

   | Layer | Checks | Codes |
   |---|---|---|
   | 1. Syntax | JSON well-formed; line and column | `RRS-E0xxx` |
   | 2. Structure | types and required fields; exact JSON paths | `RRS-E1xxx` |
   | 3. Meaning | shape inference, references resolve, ranges don't overlap, the graph is acyclic unless recurrent | `RRS-E2xxx` |
   | 4. Environment | plugins installed, fits on the GPU, adapter supports it. **Warnings, not failures** | `RRS-E3xxx` |

8. **Untrusted imports** are checked before anything is allocated: limits on size, nesting depth, node count and parameter count; zip-bomb caps (total uncompressed size and ratio); zip-slip entries (`..` or absolute paths) rejected. Codes `RRS-E5xxx`. The limits are constants at the top of the limits module. *(Note since acceptance, 2026-10-10: the zip container and its caps stay in-house; DEFLATE itself goes through `miniz_oxide` in `zip/inflate.rs`, and every in-house parser here has a fuzz target (ADR-0037).)*
9. **Missing plugins never fail a load.** A file whose `requires` names a missing plugin opens with placeholder nodes and a banner listing the plugins to install (`plugin.suggest`).
10. **Standards for everything else:** `.safetensors` and `.gguf` for weights, `.jsonl` and `.parquet` for data, `.onnx` for interop. Never pickled `.pt` (ADR-0024).

**Checked by:** `fixtures/documents/<type>/valid/` and `.../invalid/`, where each invalid file has `<name>.expected.json` with the expected codes and paths; a migration test per version step; cargo-fuzz targets on the parsers; a zip-bomb and zip-slip corpus in the import tests.

## Consequences

**Good**
- Files are readable, diffable and editable in any editor, with schema completion.
- One parser everywhere: the browser viewer, the CLI and Python can't disagree.
- Users see every problem at once, with a fix, instead of the first error only.

**Costs**
- JSON has no comments. Notes go in a `notes` field where a document type has one.
- Large graphs make large JSON files. Weights never go inside documents, so this stays manageable.
- Each schema version bump needs a migration function and a fixture.

## Alternatives considered

| Alternative | Why not |
|---|---|
| YAML or TOML | YAML is ambiguous (the "Norway problem", anchors) and slower to parse safely. TOML is weak for nested graphs. `$schema` tooling is best for JSON. |
| A binary format (protobuf, CBOR) | Not diffable or hand-editable, which defeats "be the default file format". |
| SQLite project files | Opaque to git and to review. |
| Validate-then-use with untyped maps | Every consumer re-checks, and the checks drift. |

## Links

- Spec: [§2](../spec/rastai_spec_v5.md#2-principles), [§13](../spec/rastai_spec_v5.md#13-plugins-and-ecosystem) (dependencies), [§15](../spec/rastai_spec_v5.md#15-file-formats), [§23](../spec/rastai_spec_v5.md#23-validation)
- Contracts: [§5 documents](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§10 RRS-E ranges](../architecture/contracts.md#10-error-catalog), [§12.5 fixtures](../architecture/contracts.md#12-naming-and-code-conventions)
- Related: [ADR-0004](0004-schema-ownership.md), [ADR-0024](0024-no-pickle-checkpoints.md), [ADR-0029](0029-strict-safetensors-contract.md), [ADR-0030](0030-diagnostics-catalog.md), [ADR-0037](0037-borrowed-decoders-and-parser-fuzzing.md)
