"""
The tokenizer stress corpus, shared by experiment 04 (gateway tokenizer vs llama.cpp) and experiment 07
(Rust vs Java backends): every paragraph of our spec, the whole spec, source code, and hand-picked edge cases.
"""

from pathlib import Path

from rastai_spike.prompts import SPEC_FILE

SPIKE_FOLDER = Path(__file__).resolve().parents[2]
CODE_FILES = [
    Path.home() / "Desktop" / "AITestLab-1" / "create_model.py",
    SPIKE_FOLDER / "src" / "rastai_spike" / "gemma3.py",
]

EDGE_CASES = {
    "empty": "",
    "single space": " ",
    "space run": "a" + " " * 17 + "b",
    "tabs and newlines": "line1\n\n\tindented\r\nwindows line\n\n\n",
    "leading and trailing spaces": "   leading and trailing   ",
    "numbers": "3.14159 2,147,483,647 1e-10 0x1F 007 123456789012345",
    "chinese japanese korean": "神经网络的可解释性研究。日本語のテキスト。한국어 문장.",
    "hindi": "सामाजिक नेटवर्क और कृत्रिम बुद्धिमत्ता",
    "arabic": "الشبكات العصبية والذكاء الاصطناعي",
    "emoji": "Teach it 🚀🔥 family: 👨‍👩‍👧‍👦 flags: 🇺🇸🇯🇵 skin tone: 👍🏽",
    "combining marks": "é vs é, ñ, Å vs Å",
    "zero-width characters": "zero​width‍joiner﻿byte-order-mark",
    "control characters": "bell\x07 escape \x1b[31mred\x1b[0m",
    "nul character": "before\x00after",
    "url and path": "https://huggingface.co/google/gemma-scope-2-1b-it?x=1&y=2#frag /home/snub/Desktop/RASTAI",
    "markdown table": "| a | b |\n|---|---|\n| 1 | 2 |",
    "repeated character": "=" * 300,
    "mixed case": "RaStAi ReSeArCh StUdIo",
    "special tokens as text": "<start_of_turn>user\nHi<end_of_turn>\n<start_of_turn>model\n",
    "special token look-alikes": "<start_of_turn > <bos > </s> <unk> <pad> <eos> <start_of_turn",
}


def build_corpus() -> dict[str, str]:
    spec_text = SPEC_FILE.read_text()
    corpus = {"whole spec": spec_text}
    paragraphs = [paragraph for paragraph in spec_text.split("\n\n") if paragraph.strip()]
    corpus.update({f"spec paragraph {index}": paragraph for index, paragraph in enumerate(paragraphs)})
    corpus.update({f"code: {path.name}": path.read_text() for path in CODE_FILES if path.exists()})
    corpus.update({f"edge: {name}": text for name, text in EDGE_CASES.items()})
    return corpus
