# python/rastai_data: dataset tooling

Read the root `CLAUDE.md` and `python/CLAUDE.md` first. The layout is `docs/architecture/layout/python.md` §4.

- **What is here (M1-36).** `readers/jsonl.py` and `readers/csv_reader.py` (rows as dicts), `word_vocabulary.py` (the word-level vocabulary that replaces torchtext for TextCNN), `splits.py` (`assign_split`), `loaders.py` (`epoch_order`, `make_loader`, `BatchLoader` with a resumable position), `examples.py` (the example dataclasses) and `errors.py` (`DataError`). The rest of layout §4.1 lands in its build-order step (§4.8).
- **One error class.** Every data problem the user can fix raises `DataError(code, message)` with an `RRS-D` code from contracts §10.3. A wrong argument from calling code (a batch size of 0, ratios that don't add up to 1) is a bug and raises `ValueError`.
- **Readers parse untrusted files.** A reader returns rows or raises `DataError`; never another exception, never a hang. `tests/test_reader_mutations.py` (marked `fuzz`) checks that on seeded mutations of `fixtures/datasets/` (contracts §12.5, ADR-0037). A new reader adds its fixture to that test in the same PR.
- **Deterministic everywhere.** Splits hash with `zlib.crc32`, shuffles use a private `random.Random` seeded with a string; never Python's `hash()` or the global `random`. The tests run both under two `PYTHONHASHSEED` values.
- **Standard library only for now.** Each external library joins with its one wrapper file (`tokenization.py`, `image_decode.py`, `readers/parquet.py`, `readers/mysql_reader.py`), listed in `pyproject.toml` in the same PR.
