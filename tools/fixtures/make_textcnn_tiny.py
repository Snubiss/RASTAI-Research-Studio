"""Writes fixtures/datasets/textcnn_tiny.jsonl and textcnn_tiny.csv (fixtures/datasets/README.md).

The data is ours: every sentence is a random pick from the word lists below, so no third-party text and no data
license is involved. The random generator is seeded, so running this again writes the same bytes;
tests/test_make_textcnn_tiny.py checks that against the committed files.

Usage, from the repository root:
    .dev/envs/cpu/bin/python tools/fixtures/make_textcnn_tiny.py [output_folder]
The output folder defaults to fixtures/datasets.
"""

import csv
import json
import random
import sys
from pathlib import Path

ROWS_PER_LABEL = 500
# The CSV fixture is the first rows of the JSONL fixture, so the two readers can be compared row for row.
CSV_ROW_COUNT = 200
# A string seed: random.Random hashes it the same way on every machine (unlike hash(), it isn't salted).
SEED = "textcnn_tiny/1"
DEFAULT_OUTPUT_FOLDER = Path(__file__).resolve().parents[2] / "fixtures" / "datasets"
JSONL_FILE_NAME = "textcnn_tiny.jsonl"
CSV_FILE_NAME = "textcnn_tiny.csv"
CSV_HEADER = ["id", "text", "label"]
ID_PREFIX = "tiny-"

# Openers and closers are shared by every label, so a model can't tell labels apart by punctuation or length.
# The repeated empty opener and "." closer make plain sentences the most common.
OPENERS = ["", "", "", "Honestly, ", "Today ", "I think ", "Wow, ", "So ", "Yesterday ", "Apparently "]
CLOSERS = [".", ".", ".", "!", "?", "...", ", right?", ", again.", " - really.", ", I guess."]

# Per label: a sentence is one subject, one verb phrase and one ending. The order of the labels and of every list
# is part of the seeded draw: changing any of them changes the files.
LABEL_PARTS = {
    "weather": {
        "subjects": ["the rain", "a cold wind", "the morning fog", "the storm", "the sunshine", "heavy snow",
                     "the forecast", "a warm breeze", "the thunder", "the humidity", "a frost", "the clouds"],
        "verbs": ["is coming", "kept falling", "rolled in", "cleared up", "stayed around", "moved east",
                  "will return", "got worse", "faded away", "covered", "hit"],
        "endings": ["over the hills", "all afternoon", "near the coast", "before noon", "across town",
                    "by the evening", "in the valley", "for three days", "after midnight", "on the roads"],
    },
    "sports": {
        "subjects": ["the striker", "our team", "the goalkeeper", "the coach", "the home side", "the runner",
                     "the referee", "the captain", "the tennis player", "the visitors", "the defense", "the rookie"],
        "verbs": ["scored", "won", "lost", "trained", "defended", "sprinted", "tackled", "celebrated",
                  "missed a penalty", "took the lead", "equalized"],
        "endings": ["in the final minute", "at the stadium", "during the match", "in extra time",
                    "after the whistle", "in the second half", "on the court", "at the league game",
                    "before the playoffs", "on the track"],
    },
    "cooking": {
        "subjects": ["the soup", "my grandmother", "the chef", "the bread", "this sauce", "the oven",
                     "the garlic", "a fresh salad", "the pasta", "the roast", "the dough", "the spices"],
        "verbs": ["simmered", "was baked", "needs salt", "tasted great", "burned", "was stirred", "rose",
                  "was chopped", "smelled wonderful", "cooled", "was seasoned"],
        "endings": ["in the kitchen", "for an hour", "with fresh herbs", "on low heat", "before dinner",
                    "with butter and lemon", "in a cast iron pan", "overnight", "for the guests",
                    "with olive oil"],
    },
    "music": {
        "subjects": ["the band", "the guitarist", "the choir", "the drummer", "the new album", "the singer",
                     "the orchestra", "this melody", "the piano", "the chorus", "the bass line", "the violinist"],
        "verbs": ["played", "recorded", "rehearsed", "sounded amazing", "went on tour", "was tuned",
                  "hit the high note", "released a single", "improvised", "kept the rhythm", "sang"],
        "endings": ["at the concert", "in the studio", "on the radio", "during the encore", "all night",
                    "at the festival", "in a minor key", "with a quiet intro", "for the crowd",
                    "at the jazz club"],
    },
}  # fmt: skip


def make_text(parts: dict, generator: random.Random) -> str:
    """One sentence: opener, subject, verb phrase, ending and closer, with its first letter capitalized."""
    subject = generator.choice(parts["subjects"])
    verb = generator.choice(parts["verbs"])
    ending = generator.choice(parts["endings"])
    opener = generator.choice(OPENERS)
    text = f"{opener}{subject} {verb} {ending}"
    text = text[0].upper() + text[1:]
    return text + generator.choice(CLOSERS)


def make_rows() -> list[dict]:
    """ROWS_PER_LABEL rows per label, shuffled, then numbered in their shuffled order."""
    generator = random.Random(SEED)
    rows = []
    for label, parts in LABEL_PARTS.items():
        for _ in range(ROWS_PER_LABEL):
            rows.append({"text": make_text(parts, generator), "label": label})
    generator.shuffle(rows)
    numbered_rows = []
    for index, row in enumerate(rows):
        numbered_rows.append({"id": f"{ID_PREFIX}{index:04d}", "text": row["text"], "label": row["label"]})
    return numbered_rows


def write_jsonl(rows: list[dict], path: Path) -> None:
    with open(path, "w", encoding="utf-8", newline="\n") as jsonl_file:
        for row in rows:
            jsonl_file.write(json.dumps(row, ensure_ascii=False) + "\n")


def write_csv(rows: list[dict], path: Path) -> None:
    # newline="" and lineterminator="\n": the same line endings on every operating system.
    with open(path, "w", encoding="utf-8", newline="") as csv_file:
        writer = csv.writer(csv_file, lineterminator="\n")
        writer.writerow(CSV_HEADER)
        for row in rows:
            writer.writerow([row[column] for column in CSV_HEADER])


def write_fixtures(output_folder: Path) -> None:
    rows = make_rows()
    write_jsonl(rows, output_folder / JSONL_FILE_NAME)
    write_csv(rows[:CSV_ROW_COUNT], output_folder / CSV_FILE_NAME)


def main() -> None:
    output_folder = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT_FOLDER
    write_fixtures(output_folder)


if __name__ == "__main__":
    main()
