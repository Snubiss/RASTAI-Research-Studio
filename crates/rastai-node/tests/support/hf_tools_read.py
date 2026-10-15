"""Checks a hub cache the node wrote against the layout huggingface_hub writes (contracts 8.1; layout 7.9).

The layout is recorded in hf_cache_recorded.json, from huggingface_hub 1.33.0 (the version pinned in
python/envs/cpu/uv.lock). This script uses only the standard library, so it does not import huggingface_hub and needs
no package in the environment (spec 27). Exit 0 when the cache matches the recording and the config reads through its
snapshot link, else exit 1 with the reason on stderr.
"""

import json
import os
import re
import sys

FIXTURE_NAME = "hf_cache_recorded.json"
USAGE = "usage: hf_tools_read.py <hub cache> <repo id> <commit> <config file name>"
BLOB_PATTERN = "(?:[0-9a-f]{40}|[0-9a-f]{64})"
PLACEHOLDER_SPLIT = re.compile(r"(<[a-z]+>)")
FILE_KIND = "file"
SYMLINK_KIND = "symlink"


def refuse(reason):
    print(reason, file=sys.stderr)
    sys.exit(1)


def load_fixture():
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, FIXTURE_NAME), encoding="utf-8") as handle:
        return json.load(handle)["entries"]


def placeholder_values(repo_folder, commit):
    return {
        "<repo>": re.escape(repo_folder),
        "<commit>": re.escape(commit),
        "<blob>": BLOB_PATTERN,
        "<name>": "[^/]+",
        "<nested>": "(?:[^/]+/)+[^/]+",
    }


def pattern_for(template, values):
    """A regular expression for a recorded template: its placeholders match what they stand for, all else literally."""
    pieces = []
    for part in PLACEHOLDER_SPLIT.split(template):
        pieces.append(values.get(part, re.escape(part)))
    return re.compile("".join(pieces) + "$")


def kind_of(path):
    if os.path.islink(path):
        return SYMLINK_KIND
    return FILE_KIND


def cache_files(cache):
    """Every file and symlink under the cache, as paths relative to it."""
    found = []
    for folder, _, names in os.walk(cache):
        for name in names:
            full = os.path.join(folder, name)
            found.append((os.path.relpath(full, cache), kind_of(full)))
    return found


def matching_entries(relative, kind, entries, values):
    """The recorded entries of this kind whose template matches the path."""
    hits = []
    for entry in entries:
        if entry["kind"] != kind:
            continue
        if pattern_for(entry["path"], values).match(relative):
            hits.append(entry)
    return hits


def check_unrecorded_files(cache, entries, values):
    """Every file the node wrote is one huggingface_hub writes too: a recorded template, with the right kind."""
    for relative, kind in cache_files(cache):
        if not matching_entries(relative, kind, entries, values):
            refuse(f"{relative} ({kind}) is not in the recorded huggingface_hub layout")


def check_required_entries(cache, entries, values):
    """Each required template has a file of its kind; a symlink names a blob that exists, under the recorded target."""
    for entry in entries:
        if not entry["required"]:
            continue
        pattern = pattern_for(entry["path"], values)
        found = [
            (relative, kind)
            for relative, kind in cache_files(cache)
            if kind == entry["kind"] and pattern.match(relative)
        ]
        if not found:
            refuse(f"no {entry['kind']} matches the recorded {entry['path']}")
        for relative, _ in found:
            check_entry_content(cache, relative, entry, values)


def check_entry_content(cache, relative, entry, values):
    full = os.path.join(cache, relative)
    if "content" in entry:
        expected = pattern_for(entry["content"], values)
        with open(full, encoding="utf-8") as handle:
            text = handle.read().strip()
        if not expected.match(text):
            refuse(f"{relative} holds {text!r}, not the recorded {entry['content']}")
    if "target" in entry:
        target = os.readlink(full)
        if not pattern_for(entry["target"], values).match(target):
            refuse(f"{relative} links to {target!r}, not the recorded {entry['target']}")
        if not os.path.isfile(full):
            refuse(f"{relative} links to a blob that does not exist")


def check_config(cache, repo_folder, commit, config_name):
    """The config is a snapshot link, and a loader can read its bytes through it."""
    link = os.path.join(cache, repo_folder, "snapshots", commit, config_name)
    if not os.path.islink(link):
        refuse(f"{link} is not a snapshot link")
    with open(link, "rb") as handle:
        if not handle.read():
            refuse(f"{link} reads as empty through its snapshot link")


def main(arguments):
    if len(arguments) != 4:
        refuse(USAGE)
    cache, repo_id, commit, config_name = arguments
    repo_folder = "models--" + repo_id.replace("/", "--")
    entries = load_fixture()
    values = placeholder_values(repo_folder, commit)
    check_unrecorded_files(cache, entries, values)
    check_required_entries(cache, entries, values)
    check_config(cache, repo_folder, commit, config_name)


if __name__ == "__main__":
    main(sys.argv[1:])
