# Document corpus

Golden files for `rastai-formats`' `read_document` (contracts §12.5, ADR-0032). `crates/rastai-formats/tests/documents_corpus.rs` reads every file here, and from M1-19 and M4 the wheel and WASM parity tests read the same files and must give the same codes and paths.

## Folders

| Folder | Holds |
|---|---|
| `<type>/valid/` | Files of one document type (the folder is the full type name, e.g. `rastai.model`) that must read with no error. Warnings and info are allowed. |
| `<type>/invalid/` | Files of that type that must fail, each with its expected diagnostics. |
| `<type>/v<n>/` | Files at an old schema version, for the migration test (`tests/migrations.rs`, once a type has a v2). |
| `any/invalid/` | Cases every document shares whatever its type: syntax errors, import limits, the shared header, an unknown `type`. Their `type` names no real type, so they keep their verdict as types are added. |
| `import/invalid/` | Imports that must be refused (`.pt2` with pickle, oversized configs). |

## Cases

- **One file per case**, named for what it shows (`duplicate_key.json`, `header_problems.json`). The harness passes only the file name to `read_document`, so the expected paths are the same on every machine.
- **Each invalid file has `<file name>.expected.json`** beside it, for example `nan.json.expected.json`:

  ```json
  {
    "diagnostics": [
      { "code": "RRS-E0004", "path": "nan.json:2:21" }
    ]
  }
  ```

  It lists every diagnostic of the file, in the order one read reports them: the `type` check, then the migration, then the structure walk (each object's fields in description order, then its unknown keys in key order). `path` is the full diagnostic path, `file:line:column#/json/pointer` (contracts §4.1). A missing field's pointer names the field, and its line and column are those of the object that should hold it.
- **An expected file without its fixture fails the test**, so renaming a case can't leave its expectations behind.
- **A file that isn't UTF-8** (`any/invalid/not_utf8.json`) is kept byte for byte; don't let an editor re-save it.
