"""Shared helpers for the repo checks in tools/checks (layout: rust-control-plane.md section 2.1).

Every check walks the same tree the same way, takes the same --root argument and prints its problems the same
way, so those three things live here once. Standard library only: the checks run before any environment exists.
"""

import argparse
import os
from pathlib import Path

# Folders no check ever reads, wherever they sit: version control, build output, installed packages and the
# throwaway data directory. They are the folders .gitignore leaves out, so a check sees what git would track.
SKIPPED_FOLDER_NAMES = {".git", "target", "node_modules", ".dev", ".venv", "__pycache__", "dist", "build"}
# Top-level folders no check reads: the spike is read-only history and nothing imports it (contracts section 1).
SKIPPED_TOP_FOLDERS = {"spike"}
# tools/checks/<script> sits two folders below the repository root.
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def parse_root_argument(description: str) -> Path:
    """Reads the one argument most checks take: the tree to check (the repository by default)."""
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="the tree to check (default: this repository)")
    return parser.parse_args().root


def list_files(root: Path, suffixes: tuple[str, ...]) -> list[str]:
    """Every file under root whose name ends with one of suffixes, as sorted relative paths with / separators."""
    found = []
    for folder, child_folders, file_names in os.walk(root):
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
    spans = []
    position = 0
    code_start = 0
    while position < len(text):
        kind, end = next_non_code_span(text, position)
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


def next_non_code_span(text: str, position: int) -> tuple[str | None, int]:
    """("comment" | "string", end) when a comment or string starts at position, else (None, position)."""
    if text.startswith("//", position):
        end = text.find("\n", position)
        return "comment", len(text) if end == -1 else end
    if text.startswith("/*", position):
        end = text.find("*/", position + 2)
        return "comment", len(text) if end == -1 else end + 2
    if text[position] in "\"'`":
        return "string", string_end(text, position)
    return None, position


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


def report(check_name: str, problems: list[str], passed_message: str) -> int:
    """Prints the problems, one per line, and returns the process exit code: 0 when there are none."""
    if not problems:
        print(f"{check_name}: {passed_message}")
        return 0
    for problem in problems:
        print(f"{check_name}: {problem}")
    print(f"{check_name}: {len(problems)} problem(s)")
    return 1
