"""The word vocabulary's edge cases (M1-36): tokenizing, unknown words, the maximum size and stable ids."""

import pytest
from rastai_data.readers.jsonl import read_jsonl_rows
from rastai_data.word_vocabulary import (
    PADDING_ID,
    PADDING_WORD,
    UNKNOWN_ID,
    UNKNOWN_WORD,
    WordVocabulary,
    build_word_vocabulary,
    encode_text,
    split_words,
)

TEXTS = [
    "The rain kept falling.",
    "the RAIN, the wind!",
    "Wind and rain",
]


def test_split_words_lowercases_and_drops_punctuation():
    assert split_words("Hello, World! It's 5 o'clock...") == ["hello", "world", "it's", "5", "o'clock"]


def test_split_words_keeps_letters_of_any_script():
    assert split_words("Café déjà-vu, naïve") == ["café", "déjà", "vu", "naïve"]


def test_split_words_of_text_without_words_is_empty():
    assert split_words("") == []
    assert split_words("  ... !!! -- ") == []


def test_reserved_words_come_first():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    assert vocabulary.words[UNKNOWN_ID] == UNKNOWN_WORD
    assert vocabulary.words[PADDING_ID] == PADDING_WORD


def test_words_are_ordered_by_count_then_alphabetically():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    # "rain" 3, "the" 3, "wind" 2, then the words seen once in alphabetical order.
    assert vocabulary.words == [UNKNOWN_WORD, PADDING_WORD, "rain", "the", "wind", "and", "falling", "kept"]


def test_ids_do_not_depend_on_text_order():
    forward = build_word_vocabulary(TEXTS, max_size=100)
    backward = build_word_vocabulary(list(reversed(TEXTS)), max_size=100)
    assert forward.words == backward.words


def test_max_size_keeps_the_most_frequent_words():
    vocabulary = build_word_vocabulary(TEXTS, max_size=4)
    assert vocabulary.words == [UNKNOWN_WORD, PADDING_WORD, "rain", "the"]
    assert encode_text(vocabulary, "the wind") == [3, UNKNOWN_ID]


def test_max_size_of_only_the_reserved_words_maps_everything_to_unknown():
    vocabulary = build_word_vocabulary(TEXTS, max_size=2)
    assert vocabulary.words == [UNKNOWN_WORD, PADDING_WORD]
    assert encode_text(vocabulary, "the rain") == [UNKNOWN_ID, UNKNOWN_ID]


def test_max_size_larger_than_the_word_count_keeps_every_word():
    vocabulary = build_word_vocabulary(TEXTS, max_size=1000)
    assert len(vocabulary.words) == 8


@pytest.mark.parametrize("max_size", [-1, 0, 1])
def test_max_size_below_the_reserved_words_is_refused(max_size):
    with pytest.raises(ValueError, match="max_size"):
        build_word_vocabulary(TEXTS, max_size=max_size)


def test_no_texts_give_only_the_reserved_words():
    vocabulary = build_word_vocabulary([], max_size=10)
    assert vocabulary.words == [UNKNOWN_WORD, PADDING_WORD]


def test_unknown_words_encode_to_zero():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    assert encode_text(vocabulary, "The rain and the zebra") == [3, 2, 5, 3, UNKNOWN_ID]


def test_encoding_ignores_case_like_building():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    assert encode_text(vocabulary, "RAIN Rain rain") == [2, 2, 2]


def test_empty_text_encodes_to_no_ids():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    assert encode_text(vocabulary, "") == []


def test_text_can_never_produce_a_reserved_word():
    vocabulary = build_word_vocabulary(["<unk> <pad>"], max_size=100)
    assert vocabulary.words == [UNKNOWN_WORD, PADDING_WORD, "pad", "unk"]
    assert encode_text(vocabulary, "<pad>") == [2]


def test_a_saved_word_list_rebuilds_the_same_vocabulary():
    vocabulary = build_word_vocabulary(TEXTS, max_size=100)
    rebuilt = WordVocabulary(words=list(vocabulary.words))
    assert rebuilt.ids_by_word == vocabulary.ids_by_word


def test_a_word_list_without_the_reserved_words_first_is_refused():
    with pytest.raises(ValueError, match="starts with"):
        WordVocabulary(words=["rain", UNKNOWN_WORD, PADDING_WORD])


def test_a_word_list_with_a_repeated_word_is_refused():
    with pytest.raises(ValueError, match="once"):
        WordVocabulary(words=[UNKNOWN_WORD, PADDING_WORD, "rain", "rain"])


def test_the_fixture_dataset_builds_a_vocabulary(fixtures_folder):
    rows = list(read_jsonl_rows(fixtures_folder / "datasets" / "textcnn_tiny.jsonl"))
    vocabulary = build_word_vocabulary((row["text"] for row in rows), max_size=50)
    assert len(vocabulary.words) == 50
    # Every fixture text has words, and the most common ones ("the") are in a 50-word vocabulary.
    assert all(encode_text(vocabulary, row["text"]) for row in rows)
    assert "the" in vocabulary.ids_by_word
