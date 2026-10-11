"""rastai_data: dataset tooling, from a dropped file to a training batch (layout python.md section 4)."""

from rastai_data.errors import DataError
from rastai_data.examples import LabeledExample, PreferencePair, PromptExample, TrainingExample
from rastai_data.loaders import BatchLoader, epoch_order, make_loader
from rastai_data.readers.csv_reader import read_csv_rows
from rastai_data.readers.jsonl import read_jsonl_rows
from rastai_data.splits import assign_split
from rastai_data.word_vocabulary import WordVocabulary, build_word_vocabulary, encode_text, split_words

__all__ = [
    "BatchLoader",
    "DataError",
    "LabeledExample",
    "PreferencePair",
    "PromptExample",
    "TrainingExample",
    "WordVocabulary",
    "assign_split",
    "build_word_vocabulary",
    "encode_text",
    "epoch_order",
    "make_loader",
    "read_csv_rows",
    "read_jsonl_rows",
    "split_words",
]
