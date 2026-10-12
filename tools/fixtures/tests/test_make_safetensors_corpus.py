"""tools/fixtures/make_safetensors_corpus.py writes exactly the committed safetensors corpus.

If this fails, either the generator changed or someone edited a fixture by hand. The corpus is a frozen test input
(fixtures/safetensors/README.md): a new edge case is a new file, added here and to expected.json together.
"""

from pathlib import Path

import make_safetensors_corpus

# This file sits in tools/fixtures/tests/.
REPO_ROOT = Path(__file__).resolve().parents[3]
COMMITTED_FOLDER = REPO_ROOT / "fixtures" / "safetensors"
CORPUS_SIZE = 33
VALID_COUNT = 6


def committed_files() -> list[str]:
    return sorted(path.name for path in COMMITTED_FOLDER.iterdir() if path.suffix != ".md")


def test_the_generator_writes_the_committed_bytes(tmp_path):
    make_safetensors_corpus.write_fixtures(tmp_path)
    written = sorted(path.name for path in tmp_path.iterdir())
    assert written == committed_files()
    for name in written:
        assert (tmp_path / name).read_bytes() == (COMMITTED_FOLDER / name).read_bytes(), name


def test_the_corpus_has_the_spikes_33_files_six_of_them_valid():
    cases = make_safetensors_corpus.corpus_cases()
    assert len(cases) == CORPUS_SIZE
    valid = [name for name, (_, verdict) in cases.items() if verdict["accepted"]]
    assert len(valid) == VALID_COUNT


def test_the_default_output_folder_is_the_committed_one():
    assert make_safetensors_corpus.DEFAULT_OUTPUT_FOLDER == COMMITTED_FOLDER
