# Dataset fixtures

Small datasets for the `rastai_data` tests and the TextCNN training tests (contracts §12.5 and §15.8; layout `python.md` §4.6).

| File | What it holds |
|---|---|
| `textcnn_tiny.jsonl` | 2,000 short English sentences in 4 labels (`weather`, `sports`, `cooking`, `music`), 500 each, in shuffled order. One JSON object per line: `{"id": "tiny-0000", "text": "...", "label": "weather"}`. The ids are unique and are the split keys. |
| `textcnn_tiny.csv` | the first 200 rows of `textcnn_tiny.jsonl` as CSV with the header `id,text,label`; texts that hold a comma are quoted |

**Where the data comes from.** We wrote it: every sentence is a random pick of a subject, a verb phrase and an ending from small word lists per label, with an optional opener ("Honestly, ", "Yesterday ") and closer ("!", ", right?"), drawn by a `random.Random` seeded with the string `textcnn_tiny/1`. No third-party text is in it, so no data license applies. A few words appear under more than one label ("the", "in", "at"), so a model has to learn from the label-specific words, not from punctuation or length.

**Who reads them.**

| Test | What it checks |
|---|---|
| `python/rastai_data/tests/test_jsonl.py`, `test_csv_reader.py` | both files read to the same rows |
| `python/rastai_data/tests/test_reader_mutations.py` (`fuzz`) | seeded mutations of both files give rows or `DataError`, never another exception |
| `python/rastai_data/tests/test_splits.py` | the split of every id is the same under two `PYTHONHASHSEED` values, and the split sizes follow the ratios |
| `python/rastai_data/tests/test_word_vocabulary.py` | a vocabulary built from the texts |

**Changing them.** The files are frozen test inputs; the split and mutation tests depend on their exact bytes. A new dataset case is a new file here, added to this table.
