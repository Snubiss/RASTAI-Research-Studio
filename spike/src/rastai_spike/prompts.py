"""Fixed test prompts shared by the spike experiments, so every experiment measures the same inputs."""

from pathlib import Path

from tokenizers import Tokenizer

TOKENIZER_FILE_NAME = "tokenizer.json"
BEGIN_TEXT_TOKEN = "<bos>"

# The long prompt is real prose from our own spec, cut to a length that crosses the
# 512-token sliding window three times, so sliding and global layers both get exercised.
SPEC_FILE = Path(__file__).resolve().parents[3] / "docs" / "spec" / "rastai_spec_v5.md"
LONG_PROMPT_TOKEN_COUNT = 1600

SHORT_PROMPT = "The capital of France is"
CHAT_PROMPT = (
    "<start_of_turn>user\n"
    "Explain in two sentences why the sky is blue.<end_of_turn>\n"
    "<start_of_turn>model\n"
)
CODE_PROMPT = 'def fibonacci(n):\n    """Return the nth Fibonacci number."""\n'


def load_tokenizer(model_folder: Path) -> Tokenizer:
    return Tokenizer.from_file(str(model_folder / TOKENIZER_FILE_NAME))


def encode(tokenizer: Tokenizer, text: str) -> list[int]:
    """Begin-of-text token plus the text. Special tokens written in the text (<start_of_turn>) are recognised."""
    return [tokenizer.token_to_id(BEGIN_TEXT_TOKEN)] + tokenizer.encode(text, add_special_tokens=False).ids


def build_prompts(tokenizer: Tokenizer) -> dict[str, list[int]]:
    long_ids = encode(tokenizer, SPEC_FILE.read_text())[:LONG_PROMPT_TOKEN_COUNT]
    return {
        "short": encode(tokenizer, SHORT_PROMPT),
        "chat": encode(tokenizer, CHAT_PROMPT),
        "code": encode(tokenizer, CODE_PROMPT),
        "long": long_ids,
    }
