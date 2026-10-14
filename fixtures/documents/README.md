# Document corpus

Golden files for `rastai-formats`' `read_document` (contracts §12.5, ADR-0032). `crates/rastai-formats/tests/documents_corpus.rs` reads every file here, and from M1-19 and M4 the wheel and WASM parity tests read the same files and must give the same codes and paths.

## Folders

| Folder | Holds |
|---|---|
| `<type>/valid/` | Files of one document type (the folder is the full type name, e.g. `rastai.theme`) that must read with no error. Warnings and info are allowed. The step-1 types are here from M1-17: `rastai.run_record`, `rastai.resume`, `rastai.settings`, `rastai.project`, `rastai.lock`, `rastai.plugin` and `rastai.theme`; `rastai.model` from M1-18 (the TextCNN template itself is `templates/designs/textcnn.rrsmodel`, which `tests/documents_corpus.rs` reads too). |
| `<type>/invalid/` | Files of that type that must fail, each with its expected diagnostics. |
| `<type>/v<n>/` | Files at an old schema version, for the migration test (`tests/migrations.rs`, once a type has a v2). |
| `any/invalid/` | Cases every document shares whatever its type: syntax errors, import limits, the shared header, an unknown `type`. Their `type` names no real type, so they keep their verdict as types are added. |
| `import/invalid/` | Imports that must be refused (`.pt2` with pickle, oversized configs). |
| `content_hashes.json` | The content hash (contracts §5.1) of every file in a `valid/` folder, keyed by its path below this folder: `{"format_version": 1, "hashes": {"rastai.theme/valid/night.theme.json": "blake3:..."}}`. `crates/rastai-formats/tests/round_trip.rs` and the wheel's `python/rastai_formats/tests/test_parity.py` both check against it, so Rust and Python give the same hashes. A new valid file adds its line; the Rust test's failure message prints every hash. |

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

  It lists every diagnostic of the file, in the order one read reports them: the `type` check, then the migration, then the structure walk (each object's fields in description order, then its unknown keys in key order), then the meaning checks of layer 3 (`RRS-E2xxx`, and `RRS-P0xxx` for a plugin manifest), which run only when layers 1 and 2 found no error. `path` is the full diagnostic path, `file:line:column#/json/pointer` (contracts §4.1). A missing field's pointer names the field, and its line and column are those of the object that should hold it.
- **A valid file must also come back as its typed document**, so a description that disagrees with its struct shows up here too.
- **An expected file without its fixture fails the test**, so renaming a case can't leave its expectations behind.
- **The tiny decoder specs are single layers.** `valid/tiny_gemma3.rrsmodel` and `valid/tiny_llama.rrsmodel` (M2-01) each unroll one layer, so they show the op vocabulary and the shapes, not a whole model. Gemma 3 is not complete here: its 5:1 local-to-global attention pattern (sliding layers with a 10k rope base, and every sixth layer global with a 1M base and linear scaling of 8) isn't in these files. A `repeat` stamps identical copies of one block, so a design whose layers differ needs one block per kind of layer, which no file here builds yet (M2-03).
- **A file that isn't UTF-8** (`any/invalid/not_utf8.json`) is kept byte for byte; don't let an editor re-save it.
