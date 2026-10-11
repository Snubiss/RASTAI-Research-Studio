# python/rastai_formats: rastai-formats for Python

Read the root `CLAUDE.md` and `python/CLAUDE.md` first. Every type here is defined once in Rust, in `crates/rastai-formats` (its layout is `docs/architecture/layout/rust-control-plane.md` §4; the generator is §4.9).

- **`rastai_formats/generated/` is generated** by `rastai-formats-codegen` from the type descriptions and never edited by hand: change the Rust struct and its `describe_<type>()`, then run `just codegen`. `just codegen-check` fails when it differs from a fresh run. ruff skips it.
- **One frozen dataclass per type** (keyword-only, fields in the description's order), with `<type>_from_json` and `<type>_to_json`. An optional field is `None` when its key is absent, and `to_json` leaves it out again, so a file read and written back keeps exactly its keys. A JSON key that isn't a Python name is renamed: `$schema` is `schema`, a reserved word gets a trailing underscore (`from_`).
- **No validation and no hashing in Python.** The generated functions only convert. Documents are validated, and content hashes computed, by rastai-formats itself through the wheel that M1-19 adds (contracts 2.4 rule 10).
- **Imports nothing internal** (`check-imports`): it is the bottom of the Python stack beside `rastai_proto`.
- Tests are in `tests/test_<module>.py`.
