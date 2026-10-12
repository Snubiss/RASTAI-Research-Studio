"""The wheel gives what Rust gives (layout rust-control-plane.md 4.11, "Parity across builds").

The document corpus, fixtures/documents/, goes through the wheel. An invalid file must give exactly the codes and
paths of its `<name>.expected.json`, the file crates/rastai-formats/tests/documents_corpus.rs checks Rust against.
A valid file must give no error and the content hash fixtures/documents/content_hashes.json lists, the file
crates/rastai-formats/tests/round_trip.rs checks Rust against. A refused file raises FormatsValidationError carrying
the same diagnostics as JSON.

Reading the expected files with the json module is fine here: they are test inputs, not documents the code accepts.
"""

import json
from pathlib import Path

import pytest

import rastai_formats
from rastai_formats import (
    FormatsValidationError,
    content_hash,
    generated,
    hash_bytes,
    load_model_spec,
    read_document,
    write_document,
)

DOCUMENTS_FOLDER = Path(__file__).resolve().parents[3] / "fixtures" / "documents"
VALID_FOLDER = "valid"
INVALID_FOLDER = "invalid"
EXPECTED_SUFFIX = ".expected.json"
CONTENT_HASHES_FILE = DOCUMENTS_FOLDER / "content_hashes.json"
# The name write_document and content_hash read their text under; it starts their diagnostics' paths.
DOCUMENT_TEXT_NAME = "document.json"
# The name load_model_spec reads its bytes under.
MODEL_SPEC_NAME = "model.rrsmodel"
NOT_UTF8_FILE = DOCUMENTS_FOLDER / "any" / "invalid" / "not_utf8.json"
RESUME_FILE = DOCUMENTS_FOLDER / "rastai.resume" / "valid" / "resume.json"
BAD_RESUME_FILE = DOCUMENTS_FOLDER / "rastai.resume" / "invalid" / "missing_dataloader.json"
# The reference vectors from the BLAKE3 paper and the official test suite (the same as hashing.rs).
EMPTY_HASH = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
ABC_HASH = "blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
CODE_NEWER_VERSION = "RRS-E4001"
TYPE_POINTER_SUFFIX = "#/type"


def corpus_files(folder_name: str) -> list[Path]:
    """Every fixture in the `valid` or `invalid` folders of the corpus, sorted, without the expected files."""
    files = DOCUMENTS_FOLDER.glob(f"*/{folder_name}/*")
    return sorted(path for path in files if path.is_file() and not path.name.endswith(EXPECTED_SUFFIX))


def corpus_key(path: Path) -> str:
    """A file's key in content_hashes.json: `rastai.theme/valid/night.theme.json`."""
    return path.relative_to(DOCUMENTS_FOLDER).as_posix()


def read_expected(fixture: Path) -> list[tuple[str, str]]:
    expected_file = fixture.with_name(fixture.name + EXPECTED_SUFFIX)
    expected = json.loads(expected_file.read_text(encoding="utf-8"))
    return [(item["code"], item["path"]) for item in expected["diagnostics"]]


def codes_and_paths(diagnostics: list[dict]) -> list[tuple[str, str]]:
    return [(item["code"], item["path"]) for item in diagnostics]


def renamed(pairs: list[tuple[str, str]], old_name: str, new_name: str) -> list[tuple[str, str]]:
    """The same diagnostics as if the file had been read under another name: every path starts with the file name."""
    return [(code, new_name + path.removeprefix(old_name)) for code, path in pairs]


def raised_diagnostics(error: pytest.ExceptionInfo) -> list[dict]:
    return json.loads(error.value.args[0])


def read_outcome(path: Path) -> dict:
    return json.loads(read_document(path.name, path.read_bytes()))


def golden_hashes() -> dict[str, str]:
    return json.loads(CONTENT_HASHES_FILE.read_text(encoding="utf-8"))["hashes"]


def test_the_corpus_is_there() -> None:
    assert len(corpus_files(VALID_FOLDER)) >= 7, "every step-1 type has a valid file"
    assert len(corpus_files(INVALID_FOLDER)) >= 20


@pytest.mark.parametrize("fixture", corpus_files(INVALID_FOLDER), ids=corpus_key)
def test_an_invalid_file_gives_the_codes_and_paths_rust_gives(fixture: Path) -> None:
    outcome = read_outcome(fixture)
    assert codes_and_paths(outcome["diagnostics"]) == read_expected(fixture)


@pytest.mark.parametrize("fixture", corpus_files(VALID_FOLDER), ids=corpus_key)
def test_a_valid_file_reads_with_no_error_and_its_typed_document(fixture: Path) -> None:
    outcome = read_outcome(fixture)
    severities = [item["severity"] for item in outcome["diagnostics"]]
    assert "error" not in severities
    assert outcome["read_only"] is False
    assert outcome["migrated_from_version"] == 0
    document = json.loads(outcome["document_json"])
    assert document["type"] == json.loads(fixture.read_text(encoding="utf-8"))["type"]


def test_the_golden_list_names_exactly_the_valid_files() -> None:
    assert sorted(golden_hashes()) == [corpus_key(path) for path in corpus_files(VALID_FOLDER)]


@pytest.mark.parametrize("fixture", corpus_files(VALID_FOLDER), ids=corpus_key)
def test_a_valid_file_has_the_content_hash_rust_gives(fixture: Path) -> None:
    expected = golden_hashes()[corpus_key(fixture)]
    assert content_hash(fixture.read_text(encoding="utf-8")) == expected
    # The typed document read_document returns is the same document, so it has the same hash.
    assert content_hash(read_outcome(fixture)["document_json"]) == expected


@pytest.mark.parametrize("fixture", corpus_files(VALID_FOLDER), ids=corpus_key)
def test_write_document_fills_the_hash_and_writing_again_changes_nothing(fixture: Path) -> None:
    expected = golden_hashes()[corpus_key(fixture)]
    written = write_document(fixture.read_text(encoding="utf-8"))
    assert written.endswith(b"\n")
    reread = json.loads(read_document(fixture.name, written))
    assert reread["diagnostics"] == []
    assert json.loads(reread["document_json"])["content_hash"] == expected
    assert write_document(written.decode("utf-8")) == written


def invalid_text_fixtures() -> list[Path]:
    """The invalid files that are UTF-8 text, which is what write_document and content_hash take."""
    return [path for path in corpus_files(INVALID_FOLDER) if path != NOT_UTF8_FILE]


@pytest.mark.parametrize("fixture", invalid_text_fixtures(), ids=corpus_key)
def test_a_bad_file_raises_with_its_diagnostics(fixture: Path) -> None:
    # The text is read under DOCUMENT_TEXT_NAME, whose extension names no type, so the expected diagnostics are the
    # ones read_document gives under that name. They differ from the fixture's own only for a file whose type comes
    # from its extension (a theme whose `type` isn't a string).
    text = fixture.read_text(encoding="utf-8")
    under_text_name = json.loads(read_document(DOCUMENT_TEXT_NAME, text.encode("utf-8")))
    expected = codes_and_paths(under_text_name["diagnostics"])
    assert expected
    with pytest.raises(FormatsValidationError) as write_error:
        write_document(text)
    assert codes_and_paths(raised_diagnostics(write_error)) == expected
    with pytest.raises(FormatsValidationError) as hash_error:
        content_hash(text)
    assert codes_and_paths(raised_diagnostics(hash_error)) == expected


def test_a_bad_resume_raises_with_the_diagnostics_rust_expects() -> None:
    expected = renamed(read_expected(BAD_RESUME_FILE), BAD_RESUME_FILE.name, DOCUMENT_TEXT_NAME)
    with pytest.raises(FormatsValidationError) as error:
        write_document(BAD_RESUME_FILE.read_text(encoding="utf-8"))
    assert codes_and_paths(raised_diagnostics(error)) == expected


def test_bytes_that_are_not_utf8_raise_with_their_syntax_code() -> None:
    expected = renamed(read_expected(NOT_UTF8_FILE), NOT_UTF8_FILE.name, MODEL_SPEC_NAME)
    with pytest.raises(FormatsValidationError) as error:
        load_model_spec(NOT_UTF8_FILE.read_bytes())
    assert codes_and_paths(raised_diagnostics(error)) == expected


def test_a_raised_diagnostic_has_every_wire_field() -> None:
    with pytest.raises(FormatsValidationError) as error:
        write_document("{")
    diagnostic = raised_diagnostics(error)[0]
    wire_fields = ["code", "fix_arguments_json", "fix_command_id", "fix_text", "message", "path", "severity"]
    assert sorted(diagnostic) == wire_fields
    assert diagnostic["severity"] == "error"
    assert diagnostic["message"]


def test_a_newer_document_is_not_written_or_hashed() -> None:
    newer = RESUME_FILE.read_text(encoding="utf-8").replace('"schema_version": 1', '"schema_version": 2')
    outcome = json.loads(read_document(RESUME_FILE.name, newer.encode("utf-8")))
    assert outcome["read_only"] is True
    for refused_call in (write_document, content_hash):
        with pytest.raises(FormatsValidationError) as error:
            refused_call(newer)
        codes = [item["code"] for item in raised_diagnostics(error)]
        assert CODE_NEWER_VERSION in codes


def test_another_document_type_is_not_a_model_spec() -> None:
    with pytest.raises(FormatsValidationError) as error:
        load_model_spec(RESUME_FILE.read_bytes())
    errors = [item for item in raised_diagnostics(error) if item["severity"] == "error"]
    assert errors
    assert all(item["path"].startswith(MODEL_SPEC_NAME) for item in errors)
    assert any(item["path"].endswith(TYPE_POINTER_SUFFIX) for item in errors)


def test_the_typed_document_feeds_the_generated_dataclass() -> None:
    document = json.loads(read_outcome(RESUME_FILE)["document_json"])
    resume = generated.resume_state_from_json(document)
    assert resume.step == 1500
    assert generated.resume_state_to_json(resume) == document


def test_hash_bytes_gives_the_blake3_reference_vectors() -> None:
    assert hash_bytes(b"") == EMPTY_HASH
    assert hash_bytes(b"abc") == ABC_HASH


def test_the_exception_is_one_python_exception_type() -> None:
    assert issubclass(FormatsValidationError, Exception)
    assert FormatsValidationError is rastai_formats._native.FormatsValidationError


def test_the_wrappers_call_the_native_module_each_time(monkeypatch: pytest.MonkeyPatch) -> None:
    # A test elsewhere (M1-34's safetensors delegation test) stands in for a native function the same way.
    monkeypatch.setattr(rastai_formats._native, "hash_bytes", lambda data: "stand-in")
    assert hash_bytes(b"abc") == "stand-in"
