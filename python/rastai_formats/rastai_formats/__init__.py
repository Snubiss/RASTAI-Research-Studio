"""rastai-formats for Python: every document and command type, written once in Rust (contracts 2.2 and 5.4).

generated/ is written by `rastai-formats-codegen` from the type descriptions in crates/rastai-formats and never
edited by hand: one frozen dataclass per type, with its from_json and to_json functions, all importable from
`rastai_formats.generated`. M1-19 adds the wheel's functions here (read_document, write_document, content_hash,
hash_bytes), the one place Python validates a document or computes a hash.
"""
