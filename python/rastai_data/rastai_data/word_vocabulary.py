"""A word-level vocabulary for from-scratch text models such as TextCNN (layout python.md section 4.1).

It replaces torchtext's vocabulary: split text into lowercase words, keep the most frequent ones up to a maximum
size, and map every other word to the unknown id 0.
"""

import re
from collections import Counter
from collections.abc import Iterable
from dataclasses import dataclass, field

# Every vocabulary starts with these two entries, so their ids never change with the data.
UNKNOWN_WORD = "<unk>"
PADDING_WORD = "<pad>"
UNKNOWN_ID = 0
# Batches of different-length texts are padded with this id, kept apart from the unknown id so a model can tell
# "no word here" from "a word it has never seen".
PADDING_ID = 1
RESERVED_WORDS = (UNKNOWN_WORD, PADDING_WORD)

# A word is a run of letters, digits or underscores, and may keep apostrophes inside it ("don't", "o'clock").
# Everything else (spaces, punctuation, symbols) only separates words. The reserved words contain < and >, so no
# text can ever produce them.
WORD_PATTERN = re.compile(r"\w+(?:'\w+)*")


@dataclass
class WordVocabulary:
    """`words[i]` is the word with id `i`. `ids_by_word` is the reverse map, built from `words`."""

    words: list[str]
    ids_by_word: dict[str, int] = field(init=False, repr=False)

    def __post_init__(self) -> None:
        check_vocabulary_words(self.words)
        self.ids_by_word = {word: word_id for word_id, word in enumerate(self.words)}


def check_vocabulary_words(words: list[str]) -> None:
    """Refuses a word list that isn't a vocabulary: the reserved words must come first, and no word twice."""
    if tuple(words[: len(RESERVED_WORDS)]) != RESERVED_WORDS:
        raise ValueError(f"a word vocabulary starts with {list(RESERVED_WORDS)}")
    if len(set(words)) != len(words):
        raise ValueError("a word vocabulary lists each word once")


def split_words(text: str) -> list[str]:
    """The basic tokenizer: lowercase, then the words in order. Punctuation is dropped."""
    return WORD_PATTERN.findall(text.lower())


def build_word_vocabulary(texts: Iterable[str], max_size: int) -> WordVocabulary:
    """The reserved words, then the most frequent words of `texts`, `max_size` entries at most in all.

    Words with the same count are ordered alphabetically, so the same texts give the same ids in any order.
    """
    if max_size < len(RESERVED_WORDS):
        raise ValueError(f"max_size must be at least {len(RESERVED_WORDS)}, for the reserved words; got {max_size}")
    counts = count_words(texts)
    ranked_words = sorted(counts, key=lambda word: (-counts[word], word))
    kept_words = ranked_words[: max_size - len(RESERVED_WORDS)]
    return WordVocabulary(words=list(RESERVED_WORDS) + kept_words)


def count_words(texts: Iterable[str]) -> Counter:
    counts = Counter()
    for text in texts:
        counts.update(split_words(text))
    return counts


def encode_text(vocabulary: WordVocabulary, text: str) -> list[int]:
    """The id of each word of `text`; a word the vocabulary doesn't have is UNKNOWN_ID."""
    word_ids = []
    for word in split_words(text):
        word_ids.append(vocabulary.ids_by_word.get(word, UNKNOWN_ID))
    return word_ids
