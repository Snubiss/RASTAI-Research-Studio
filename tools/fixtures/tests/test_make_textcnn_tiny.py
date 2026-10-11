"""tools/fixtures/make_textcnn_tiny.py writes exactly the committed dataset fixtures.

If this fails, either the generator changed or someone edited a fixture by hand. The fixtures are frozen test
inputs (fixtures/datasets/README.md), so the fix is to undo the change, not to regenerate.
"""

from pathlib import Path

import pytest

import make_textcnn_tiny

# This file sits in tools/fixtures/tests/.
REPO_ROOT = Path(__file__).resolve().parents[3]
COMMITTED_FOLDER = REPO_ROOT / "fixtures" / "datasets"


@pytest.mark.parametrize("file_name", [make_textcnn_tiny.JSONL_FILE_NAME, make_textcnn_tiny.CSV_FILE_NAME])
def test_the_generator_writes_the_committed_bytes(tmp_path, file_name):
    make_textcnn_tiny.write_fixtures(tmp_path)
    assert (tmp_path / file_name).read_bytes() == (COMMITTED_FOLDER / file_name).read_bytes()


def test_the_default_output_folder_is_the_committed_one():
    assert make_textcnn_tiny.DEFAULT_OUTPUT_FOLDER == COMMITTED_FOLDER
