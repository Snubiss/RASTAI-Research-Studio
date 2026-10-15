"""The three ways to run a registry command, each turned into one `Outcome` so the test can compare them:

- `HttpSurface`: `CommandService.Execute` as Connect JSON over the loopback port, with a bearer token, the way any
  HTTP client calls the daemon (layout: rust-control-plane.md 11.3).
- `CliSurface`: the `rastai` verb with flags and `--json`, over daemon.sock as the owner (layout 9.3).
- `McpSurface`: a `tools/call` through the `rastai mcp` stdio bridge with `RASTAI_TOKEN` (layout 7.3). It records
  every message it exchanges, so the session can be compared with the golden transcript.

Each surface shows its answer its own way: HTTP the `ExecuteResponse`, the CLI its result line and the final
`{"exit_code", "diagnostics"}` line, MCP `structuredContent` or `isError` text. A job command is answered with its
job id on all three; only HTTP also carries the result (`run_id` and the plan), which the CLI and MCP leave to
`job.get` (layout 9.3 step 6, 7.3)."""

import json
import queue
import subprocess
import threading
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

from daemon_process import surface_environment

EXECUTE_URL = "http://127.0.0.1:{port}/rastai.commands.v1.CommandService/Execute"
REQUEST_LIMIT_SECONDS = 60
# One verb or one MCP call answers at once; a job command answers before its job runs (flows/platform.md 3.1).
CLI_LIMIT_SECONDS = 120
MCP_REPLY_LIMIT_SECONDS = 120
# The commands of `kind: job`, answered with a job id. The CLI's `--detach` prints the id instead of following.
JOB_COMMANDS = ("train.start",)
# Command ids become CLI words segment by segment, with `_` written `-` (contracts 6.1), and MCP tool names with
# `.` written `__` (flows/platform.md 4.3).
COMMAND_ID_SEPARATOR = "."
CLI_WORD_SEPARATOR = "-"
SEGMENT_WORD_SEPARATOR = "_"
MCP_TOOL_SEPARATOR = "__"
TOKEN_VARIABLE = "RASTAI_TOKEN"
MCP_PROTOCOL_VERSION = "2025-06-18"
MCP_IDEMPOTENCY_KEY_MEMBER = "idempotencyKey"
MCP_CLIENT = {"name": "three-surfaces-test", "version": "1"}
# An MCP error item is `<code>: <message>`, the Basic text, then `Fix: <fix_text>` when there is a fix
# (rastai-mcp's diagnostic_text, rastai-diag's render_basic).
MCP_CODE_SEPARATOR = ": "
MCP_FIX_LABEL = "Fix: "
# The fields every surface shows of a diagnostic. HTTP and the CLI also show the fix command and the path.
COMMON_DIAGNOSTIC_FIELDS = ("code", "message", "fix_text")
FULL_DIAGNOSTIC_FIELDS = ("code", "message", "fix_text", "fix_command_id", "path")


@dataclass
class Outcome:
    """What one surface answered for one command."""

    surface: str
    ok: bool
    # A sync command's result. A job command's result only over HTTP (see the top), and None for a refusal.
    result: dict | None
    # The job a job command started; empty otherwise.
    job_id: str
    # Each diagnostic with the fields this surface shows (COMMON_DIAGNOSTIC_FIELDS, or FULL_DIAGNOSTIC_FIELDS).
    diagnostics: list[dict]
    # The CLI's exit code; None on the other surfaces.
    exit_code: int | None = None


class HttpSurface:
    name = "http"

    def __init__(self, port: int, token: str):
        self.url = EXECUTE_URL.format(port=port)
        self.token = token

    def execute(self, command_id: str, arguments: dict, idempotency_key: str = "") -> Outcome:
        request_body = {"command_id": command_id, "arguments_json": json.dumps(arguments)}
        if idempotency_key:
            request_body["idempotency_key"] = idempotency_key
        response = self.post(request_body)
        result_json = response.get("result_json", "")
        return Outcome(
            surface=self.name,
            ok=response.get("ok", False),
            result=json.loads(result_json) if result_json else None,
            job_id=response.get("job_id", ""),
            diagnostics=[full_diagnostic(item) for item in response.get("diagnostics", [])],
        )

    def post(self, request_body: dict) -> dict:
        """A refusal is still HTTP 200 with `ok: false` (flows/platform.md 3.1); only a credential problem isn't,
        and that raises here."""
        request = urllib.request.Request(
            self.url,
            data=json.dumps(request_body).encode(),
            method="POST",
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.token}"},
        )
        with urllib.request.urlopen(request, timeout=REQUEST_LIMIT_SECONDS) as response:
            return json.loads(response.read())


class CliSurface:
    name = "cli"

    def __init__(self, binary: Path, home: Path):
        self.binary = binary
        self.home = home

    def execute(self, command_id: str, arguments: dict, idempotency_key: str = "") -> Outcome:
        command_line = [str(self.binary), *cli_words(command_id), *cli_flags(arguments), "--json", "--no-start"]
        if idempotency_key:
            command_line += ["--idempotency-key", idempotency_key]
        if command_id in JOB_COMMANDS:
            command_line.append("--detach")
        completed = subprocess.run(
            command_line,
            cwd=self.home,
            env=surface_environment(self.home),
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=CLI_LIMIT_SECONDS,
            check=False,
        )
        return cli_outcome(command_id, completed)


def cli_words(command_id: str) -> list[str]:
    segments = command_id.split(COMMAND_ID_SEPARATOR)
    return [segment.replace(SEGMENT_WORD_SEPARATOR, CLI_WORD_SEPARATOR) for segment in segments]


def cli_flags(arguments: dict) -> list[str]:
    """Every argument as a flag, the way "Copy as CLI" writes a command (layout 9.4). The commands this test runs
    take text arguments only."""
    flags = []
    for name, value in arguments.items():
        if not isinstance(value, str):
            raise ValueError(f"{name}: only text arguments are written as flags here, got {value!r}")
        flag = "--" + name.replace(SEGMENT_WORD_SEPARATOR, CLI_WORD_SEPARATOR)
        flags += [flag, value]
    return flags


def cli_outcome(command_id: str, completed: subprocess.CompletedProcess) -> Outcome:
    """`--json` prints one object per line: the result (or `{"job_id"}` with `--detach`), then always
    `{"exit_code", "diagnostics"}` last (layout 9.7)."""
    lines = completed.stdout.splitlines()
    if not lines:
        raise AssertionError(f"rastai printed nothing; exit {completed.returncode}, stderr:\n{completed.stderr}")
    final_line = json.loads(lines[-1])
    if final_line["exit_code"] != completed.returncode:
        raise AssertionError(f"the final line says {final_line['exit_code']}, the process {completed.returncode}")
    first_line = json.loads(lines[0]) if len(lines) > 1 else None
    is_job = command_id in JOB_COMMANDS
    return Outcome(
        surface="cli",
        ok=completed.returncode == 0,
        result=first_line if first_line is not None and not is_job else None,
        job_id=first_line["job_id"] if first_line is not None and is_job else "",
        diagnostics=[full_diagnostic(item) for item in final_line["diagnostics"]],
        exit_code=completed.returncode,
    )


def full_diagnostic(item: dict) -> dict:
    """A diagnostic as HTTP and the CLI show it. Connect JSON leaves out a field at its default, the CLI doesn't."""
    return {name: item.get(name, "") for name in FULL_DIAGNOSTIC_FIELDS}


@dataclass
class McpExchange:
    """One message the test sent, and the reply (None for a notification)."""

    client: dict
    server: dict | None


@dataclass
class McpSurface:
    """One `rastai mcp` process for one token, initialized, for the whole session."""

    binary: Path
    home: Path
    token: str
    log_path: Path
    name: str = "mcp"
    exchanges: list[McpExchange] = field(default_factory=list)
    process: subprocess.Popen | None = None
    replies: queue.Queue = field(default_factory=queue.Queue)
    next_request_id: int = 1

    def start(self) -> None:
        environment = surface_environment(self.home)
        environment[TOKEN_VARIABLE] = self.token
        with open(self.log_path, "wb") as log:
            # Its own process group, so the cleanup's group kill never reaches the test.
            self.process = subprocess.Popen(
                [str(self.binary), "mcp"],
                cwd=self.home,
                env=environment,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=log,
                text=True,
                start_new_session=True,
            )
        # stdout is read on its own thread, so every wait for a reply can have a time limit.
        threading.Thread(target=read_lines, args=(self.process.stdout, self.replies), daemon=True).start()
        params = {"protocolVersion": MCP_PROTOCOL_VERSION, "capabilities": {}, "clientInfo": MCP_CLIENT}
        self.request("initialize", params)
        self.notify("notifications/initialized")

    def stop(self) -> None:
        if self.process is None or self.process.poll() is not None:
            return
        self.process.stdin.close()  # the bridge ends at the end of its input
        try:
            self.process.wait(timeout=MCP_REPLY_LIMIT_SECONDS)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def execute(self, command_id: str, arguments: dict, idempotency_key: str = "") -> Outcome:
        tool_name = command_id.replace(COMMAND_ID_SEPARATOR, MCP_TOOL_SEPARATOR)
        params = {"name": tool_name, "arguments": arguments}
        if idempotency_key:
            # The retry key travels in `_meta` (contracts 4.7), as MCP keeps protocol metadata there.
            params["_meta"] = {MCP_IDEMPOTENCY_KEY_MEMBER: idempotency_key}
        reply = self.request("tools/call", params)
        return mcp_outcome(command_id, reply)

    def list_tools(self) -> list[dict]:
        return self.request("tools/list", {})["result"]["tools"]

    def request(self, method: str, params: dict) -> dict:
        message = {"jsonrpc": "2.0", "id": self.next_request_id, "method": method, "params": params}
        self.next_request_id += 1
        self.send(message)
        try:
            reply = self.replies.get(timeout=MCP_REPLY_LIMIT_SECONDS)
        except queue.Empty:
            raise AssertionError(f"rastai mcp didn't answer {method} within {MCP_REPLY_LIMIT_SECONDS} s") from None
        if reply is None:
            raise AssertionError(f"rastai mcp ended before answering {method}; see {self.log_path}")
        answer = json.loads(reply)
        self.exchanges.append(McpExchange(client=message, server=answer))
        return answer

    def notify(self, method: str) -> None:
        message = {"jsonrpc": "2.0", "method": method}
        self.send(message)
        self.exchanges.append(McpExchange(client=message, server=None))

    def send(self, message: dict) -> None:
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()


def read_lines(stream, lines: queue.Queue) -> None:
    """Hands each line on; None at the end of the stream, so a waiting request fails at once rather than timing out."""
    for line in stream:
        lines.put(line)
    lines.put(None)


def mcp_outcome(command_id: str, reply: dict) -> Outcome:
    result = reply["result"]
    if result["isError"]:
        diagnostics = [mcp_diagnostic(item["text"]) for item in result["content"]]
        return Outcome(surface="mcp", ok=False, result=None, job_id="", diagnostics=diagnostics)
    structured = result["structuredContent"]
    if command_id in JOB_COMMANDS:
        return Outcome(surface="mcp", ok=True, result=None, job_id=structured["job_id"], diagnostics=[])
    return Outcome(surface="mcp", ok=True, result=structured, job_id="", diagnostics=[])


def mcp_diagnostic(text: str) -> dict:
    """`RRS-C1005: <message>`, the Basic text, then `Fix: <fix_text>` when there is one."""
    lines = text.split("\n")
    code, message = lines[0].split(MCP_CODE_SEPARATOR, 1)
    last_line = lines[-1]
    has_fix = len(lines) > 2 and last_line.startswith(MCP_FIX_LABEL)
    fix_text = last_line[len(MCP_FIX_LABEL) :] if has_fix else ""
    return {"code": code, "message": message, "fix_text": fix_text}


def common_diagnostics(outcome: Outcome) -> list[dict]:
    """The diagnostics with only the fields every surface shows."""
    return [{name: item[name] for name in COMMON_DIAGNOSTIC_FIELDS} for item in outcome.diagnostics]
