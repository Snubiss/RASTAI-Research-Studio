"""The golden MCP transcripts of fixtures/mcp/e2e/: what a session through the real `rastai mcp` sent and got back,
with the values that differ from run to run replaced by placeholders, so a session compares exactly with its file
(the format is in fixtures/mcp/e2e/README.md)."""

import json
import re
from pathlib import Path

from surfaces import McpExchange

# Ids are a prefix and Crockford base32 (contracts 15.4); every run makes new ones.
ID_PLACEHOLDERS = (
    (re.compile(r"\bjob_[0-9A-Z]{20,}"), "<job_id>"),
    (re.compile(r"\brun_[0-9A-Z]{20,}"), "<run_id>"),
    (re.compile(r"\btok_[0-9A-Z]{20,}"), "<token_id>"),
    (re.compile(r"\bnode_[0-9A-Z]{20,}"), "<node_id>"),
)
REPO_PLACEHOLDER = "<repo>"
# Values every release or every heartbeat changes, by the key that holds them. `serverInfo.version` is the app
# version too (the same placeholder as fixtures/mcp/server/).
VOLATILE_KEYS = {
    "app_version": "<app_version>",
    "last_heartbeat_unix_ms": "<unix_ms>",
}
SERVER_INFO_KEY = "serverInfo"
SERVER_INFO_VERSION_KEY = "version"
APP_VERSION_PLACEHOLDER = "<app_version>"
# A tool result's text item holds the result as compact JSON (serde_json), which is normalized like the rest.
JSON_OBJECT_START = "{"
COMPACT_SEPARATORS = (",", ":")


def normalized(value, repo_root: Path):
    """`value` with every run-specific value replaced by its placeholder."""
    if isinstance(value, dict):
        return normalized_object(value, repo_root)
    if isinstance(value, list):
        return [normalized(item, repo_root) for item in value]
    if isinstance(value, str):
        return normalized_text(value, repo_root)
    return value


def normalized_object(value: dict, repo_root: Path) -> dict:
    result = {}
    for key, item in value.items():
        if key in VOLATILE_KEYS:
            result[key] = VOLATILE_KEYS[key]
        elif key == SERVER_INFO_KEY and isinstance(item, dict):
            server_info = normalized(item, repo_root)
            server_info[SERVER_INFO_VERSION_KEY] = APP_VERSION_PLACEHOLDER
            result[key] = server_info
        else:
            result[key] = normalized(item, repo_root)
    return result


def normalized_text(text: str, repo_root: Path) -> str:
    embedded = embedded_json_object(text)
    if embedded is not None:
        return json.dumps(normalized(embedded, repo_root), separators=COMPACT_SEPARATORS, ensure_ascii=False)
    text = text.replace(str(repo_root), REPO_PLACEHOLDER)
    for pattern, placeholder in ID_PLACEHOLDERS:
        text = pattern.sub(placeholder, text)
    return text


def embedded_json_object(text: str) -> dict | None:
    """The object a text item holds as JSON, or None for any other text (a diagnostic, a note)."""
    if not text.startswith(JSON_OBJECT_START):
        return None
    try:
        value = json.loads(text)
    except json.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


def recorded_lines(exchanges: list[McpExchange], repo_root: Path) -> list[dict]:
    """The session as transcript lines, without the notes the golden file adds for people."""
    return [
        {"client": normalized(exchange.client, repo_root), "server": normalized(exchange.server, repo_root)}
        for exchange in exchanges
    ]


def read_golden(path: Path) -> tuple[dict, list[dict]]:
    """The session line, and the exchanges without their notes."""
    lines = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    session = lines[0]["session"]
    exchanges = [{"client": line["client"], "server": line["server"]} for line in lines[1:]]
    return session, exchanges


def as_jsonl(lines: list[dict]) -> str:
    """The lines as a transcript file would hold them, for a failure message to show what this run recorded."""
    return "\n".join(json.dumps(line, separators=COMPACT_SEPARATORS, ensure_ascii=False) for line in lines)
