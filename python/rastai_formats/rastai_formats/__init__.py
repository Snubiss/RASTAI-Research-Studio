"""rastai-formats for Python: every document and command type, written once in Rust (contracts 2.2 and 5.4).

generated/ is written by `rastai-formats-codegen` from the type descriptions in crates/rastai-formats and never
edited by hand: one frozen dataclass per type, with its from_json and to_json functions, all importable from
`rastai_formats.generated`.

The functions below are the one place Python validates a document or computes a hash (python layout 1.4 rules 4 and
12). Each one calls the PyO3 module `rastai_formats._native` (crates/rastai-formats/bindings/python), so Python gets
exactly the codes, paths and hashes Rust gives. They look the native function up on every call, so a test can stand
in for one with `monkeypatch.setattr(rastai_formats._native, ...)`.

A refusal raises `FormatsValidationError`; its one argument is the diagnostics as a JSON array, each object with the
fields of `rastai.common.v1.Diagnostic` (`code`, `severity`, `message`, `path`, `fix_text`, `fix_command_id`,
`fix_arguments_json`).
"""

from rastai_formats import _native
from rastai_formats._native import FormatsValidationError

__all__ = [
    "FormatsValidationError",
    "content_hash",
    "hash_bytes",
    "load_model_spec",
    "read_document",
    "write_document",
]


def read_document(file_name: str, data: bytes) -> str:
    """Reads a document's bytes through all the validation layers and returns JSON text with `document_json` (the
    typed document as JSON text, empty when it was refused), `diagnostics`, `read_only` and `migrated_from_version`.
    A bad file doesn't raise: its diagnostics are the answer. `file_name` names the type by its extension and starts
    every diagnostic path; nothing is opened by name."""
    return _native.read_document(file_name, data)


def load_model_spec(json_bytes: bytes) -> str:
    """The ModelSpec in `json_bytes`, validated, migrated and written as canonical JSON, the one form the compiler
    parses. Raises FormatsValidationError for any error, or for a document that isn't a ModelSpec."""
    return _native.load_model_spec(json_bytes)


def write_document(document_json: str) -> bytes:
    """The bytes of a document's file: validated, its content hash filled in, in the stable pretty form. Raises
    FormatsValidationError when the document has an error or is from a newer schema version."""
    return _native.write_document(document_json)


def content_hash(document_json: str) -> str:
    """A validated document's content hash, `blake3:<64 hex>`: the one write_document writes into it. Raises
    FormatsValidationError like write_document."""
    return _native.content_hash(document_json)


def hash_bytes(data: bytes) -> str:
    """The BLAKE3 hash of some bytes, `blake3:<64 hex>`."""
    return _native.hash_bytes(data)
