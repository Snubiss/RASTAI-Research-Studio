"""Shared helpers for the repo checks in tools/checks (layout: rust-control-plane.md section 2.1).

Every check walks the same tree the same way, takes the same --root argument and prints its problems the same
way, so those things live here once, with the small source scanners several checks share. Standard library only:
`just` runs the checks with the cpu environment's Python, but they import nothing from it, so any Python 3.12 can
run them.
"""

import argparse
import os
import re
from collections.abc import Callable
from pathlib import Path

# Folders no check ever reads, wherever they sit: version control, build output, installed packages and the
# throwaway data directory. They are the folders .gitignore leaves out, so a check sees what git would track.
SKIPPED_FOLDER_NAMES = {".git", "target", "node_modules", ".dev", ".venv", "__pycache__", "dist", "build"}
# Top-level folders no check reads: the spike is read-only history and nothing imports it (contracts section 1).
SKIPPED_TOP_FOLDERS = {"spike"}
# tools/checks/<script> sits two folders below the repository root.
REPO_ROOT = Path(__file__).resolve().parent.parent.parent
# Any character but a line break: blanking a span keeps its line breaks, so line numbers stay right.
NOT_LINE_BREAK = re.compile(r"[^\n]")

# Variables that tell git which repository to use. Git sets them for a hook or `git rebase -x` in a linked
# worktree; left in place they would override `git -C`, so a check would read some other repository. The checks that
# run git (check-dco, check-schema-diff) clear them with git_environment().
REPOSITORY_LOCATION_VARIABLES = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
]

# The characters that can start a Rust comment, string or char literal; any other character is plain code.
RUST_SPAN_START_CHARACTERS = set("/\"'rbc")
# A Rust char literal: 'a', '"', '\n', '\'', '\x7f', '\u{1F600}'. A quote that doesn't start one is a lifetime ('a).
RUST_CHAR_LITERAL = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F]{1,6}\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'")
# The start of a raw string: r"..", r#".."#, r##".."## and so on, and the byte and C forms br".." and cr"..".
RUST_RAW_STRING_START = re.compile(r"[bc]?r(#*)\"")
# The keywords a module specifier follows with no space in between (import{X}from"x", import"x").
IMPORT_KEYWORD_BEFORE_QUOTE = re.compile(r"(?<![\w$])(?:from|import)\Z")
# The longer of the two keywords, which sets how far back follows_an_import_keyword looks.
LONGEST_IMPORT_KEYWORD_LENGTH = len("import")


def parse_root_argument(description: str) -> Path:
    """Reads the one argument most checks take: the tree to check (the repository by default)."""
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="the tree to check (default: this repository)")
    return parser.parse_args().root


def list_files(root: Path, suffixes: tuple[str, ...]) -> list[str]:
    """Every file under root whose name ends with one of suffixes, as sorted relative paths with / separators."""
    found = []
    for folder, child_folders, file_names in os.walk(root):
        # os.walk descends into whatever child_folders holds afterwards, so it must be changed in place (a new
        # list wouldn't be seen); that is how skipped folders are never read at all.
        child_folders[:] = [name for name in child_folders if should_enter(root, Path(folder) / name)]
        for name in file_names:
            if name.endswith(suffixes):
                found.append(relative_path(root, Path(folder) / name))
    return sorted(found)


def list_files_under(root: Path, folders: list[str], suffixes: tuple[str, ...]) -> list[str]:
    """Like list_files, but only inside the given folders (relative paths ending in /)."""
    return [path for path in list_files(root, suffixes) if path.startswith(tuple(folders))]


def should_enter(root: Path, folder: Path) -> bool:
    if folder.name in SKIPPED_FOLDER_NAMES:
        return False
    return relative_path(root, folder) not in SKIPPED_TOP_FOLDERS


def relative_path(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def read_text(root: Path, path: str) -> str:
    """A source file's text. Undecodable bytes become U+FFFD, so a stray byte never hides the rest of the file."""
    return (root / path).read_text(encoding="utf-8", errors="replace")


def line_number_at(text: str, offset: int) -> int:
    """The 1-based line that holds the character at offset."""
    return text.count("\n", 0, offset) + 1


def split_javascript(text: str) -> list[tuple[str, int, int]]:
    """Cuts JavaScript source into ("code" | "string" | "comment", start, end) spans, in order.

    A plain left-to-right scan: comments start with // or /*, strings with a quote or a backtick. It is enough
    for the checks (colors live in strings, JSDoc type imports in comments) and needs no parser. A regular
    expression literal holding a quote can confuse it; our code has none, and a confused span only means a
    check reads a few characters as the wrong kind.
    """
    return split_source(text, next_javascript_span)


def javascript_code_only(text: str) -> str:
    """JavaScript source with every comment and string blanked to spaces, the same length and the same line breaks.

    A check that looks for syntax (`import.meta.glob(`, `base:`) reads this, so a pattern in a string or a
    commented-out call can neither hide code nor count as code. A check that needs a quoted key reads the text itself,
    since its string is blanked here.
    """
    pieces = []
    for kind, start, end in split_javascript(text):
        piece = text[start:end]
        pieces.append(piece if kind == "code" else blank_out(piece))
    return "".join(pieces)


def split_rust(text: str) -> list[tuple[str, int, int]]:
    """Cuts Rust source into ("code" | "string" | "comment", start, end) spans, in order: // and nested /* */
    comments, "..." strings with escapes, raw strings (r#"..."#) and char literals. Lifetimes stay code."""
    return split_source(text, next_rust_span)


def rust_code_only(text: str) -> str:
    """Rust source with every comment and string blanked to spaces, the same length and the same line breaks.

    A check that looks for syntax (`unsafe {`, `blake3::`) reads this, so a URL in a string ("http://...") or a
    commented-out line can neither hide code nor count as code.
    """
    pieces = []
    for kind, start, end in split_rust(text):
        piece = text[start:end]
        pieces.append(piece if kind == "code" else blank_out(piece))
    return "".join(pieces)


def blank_out(text: str) -> str:
    """Spaces in place of every character but line breaks, so offsets and line numbers still match."""
    return NOT_LINE_BREAK.sub(" ", text)


def split_source(text: str, next_span: Callable[[str, int], tuple[str | None, int]]) -> list[tuple[str, int, int]]:
    """The scan both languages share. next_span says whether a comment or string starts at a position, and
    where it ends; everything between those spans is code."""
    spans = []
    position = 0
    code_start = 0
    while position < len(text):
        kind, end = next_span(text, position)
        if kind is None:
            position += 1
            continue
        if code_start < position:
            spans.append(("code", code_start, position))
        spans.append((kind, position, end))
        position = end
        code_start = end
    if code_start < len(text):
        spans.append(("code", code_start, len(text)))
    return spans


# ---- JavaScript ----


def next_javascript_span(text: str, position: int) -> tuple[str | None, int]:
    """("comment" | "string", end) when a comment or string starts at position, else (None, position)."""
    if text.startswith("//", position):
        return "comment", line_end(text, position)
    if text.startswith("/*", position):
        end = text.find("*/", position + 2)
        return "comment", len(text) if end == -1 else end + 2
    if text[position] == "`":
        return "string", string_end(text, position)
    if text[position] in "\"'" and (not follows_a_word(text, position) or follows_an_import_keyword(text, position)):
        return "string", string_end(text, position)
    return None, position


def follows_a_word(text: str, position: int) -> bool:
    """Whether a letter, digit or _ sits right before position. A quote string never opens there in JavaScript
    (only a backtick does, for a tagged template), so a quote after a word is JSX text, like the apostrophe in
    <p>Don't</p>, and not a string that runs to the end of the line. Rust raw strings follow the same rule."""
    return position > 0 and (text[position - 1].isalnum() or text[position - 1] == "_")


def follows_an_import_keyword(text: str, position: int) -> bool:
    """Whether the whole word `from` or `import` sits right before position, as in import{X}from"x": the minified
    and prettier-ignored forms write no space before the quote, and that quote is still a module specifier. A
    longer word that ends in those letters (reimport) is not the keyword, so its quote stays JSX text.

    Known false positive: JSX text that ends in the whole word from or import (<p>Copy from"here"</p>) reads as a
    specifier. The check fails safe on it (the word is refused as a library), and the repository has no such text.
    """
    # The window holds one character more than the longest keyword, so the lookbehind can see the character before a
    # keyword that starts inside it.
    window = text[max(0, position - LONGEST_IMPORT_KEYWORD_LENGTH - 1) : position]
    return IMPORT_KEYWORD_BEFORE_QUOTE.search(window) is not None


def string_end(text: str, start: int) -> int:
    """The offset just past the string that starts at start. A quote string also ends at a line break."""
    quote = text[start]
    position = start + 1
    while position < len(text):
        character = text[position]
        if character == "\\":
            position += 2
            continue
        if character == quote:
            return position + 1
        if character == "\n" and quote != "`":
            return position
        position += 1
    return len(text)


# ---- Rust ----


def next_rust_span(text: str, position: int) -> tuple[str | None, int]:
    """("comment" | "string", end) when a comment, string or char literal starts at position, else (None, position)."""
    if text[position] not in RUST_SPAN_START_CHARACTERS:
        return None, position
    if text.startswith("//", position):
        return "comment", line_end(text, position)
    if text.startswith("/*", position):
        return "comment", rust_block_comment_end(text, position)
    if text[position] == '"':
        return "string", rust_string_end(text, position + 1)
    if text[position] == "'":
        char_literal = RUST_CHAR_LITERAL.match(text, position)
        return ("string", char_literal.end()) if char_literal else (None, position)
    raw_start = RUST_RAW_STRING_START.match(text, position)
    if raw_start and not follows_a_word(text, position):
        closing = '"' + raw_start.group(1)
        end = text.find(closing, raw_start.end())
        return "string", len(text) if end == -1 else end + len(closing)
    return None, position


def rust_block_comment_end(text: str, start: int) -> int:
    """The offset just past the /* comment */ that starts at start. Rust block comments nest."""
    depth = 0
    position = start
    while position < len(text):
        if text.startswith("/*", position):
            depth += 1
            position += 2
        elif text.startswith("*/", position):
            depth -= 1
            position += 2
            if depth == 0:
                return position
        else:
            position += 1
    return len(text)


def rust_string_end(text: str, position: int) -> int:
    """The offset just past the "..." string whose text starts at position. Rust strings may span lines."""
    while position < len(text):
        if text[position] == "\\":
            position += 2
            continue
        if text[position] == '"':
            return position + 1
        position += 1
    return len(text)


def line_end(text: str, position: int) -> int:
    end = text.find("\n", position)
    return len(text) if end == -1 else end


def git_environment() -> dict[str, str]:
    """This process's environment without the repository variables, so git finds the repository from -C alone."""
    environment = {}
    for name, value in os.environ.items():
        if name not in REPOSITORY_LOCATION_VARIABLES:
            environment[name] = value
    return environment


def report(check_name: str, problems: list[str], passed_message: str) -> int:
    """Prints the problems, one per line, and returns the process exit code: 0 when there are none."""
    if not problems:
        print(f"{check_name}: {passed_message}")
        return 0
    for problem in problems:
        print(f"{check_name}: {problem}")
    print(f"{check_name}: {len(problems)} problem(s)")
    return 1
