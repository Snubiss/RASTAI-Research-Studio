# MCP server transcripts

Each `.jsonl` file is one MCP session against the server side of `rastai-mcp`, replayed by
`crates/rastai-mcp/tests/server_transcripts.rs` through `parse_message` and `handle_message`, the way the daemon's
`/mcp` handler runs them (layout: rust-control-plane.md 7.3). Every reply must equal the expected one exactly.

## Format

The first line sets up the session. It always runs as a scoped token (`tok_01JA2B3C4D5E6F7G8H9J0K1M2N`), as
`rastai mcp` does:

| Key | Meaning |
|---|---|
| `session.token_preset` | the token's scopes are a preset (contracts 6.3): `mcp-default` = `read`, `chat`, `project` |
| `session.scopes` | or the scopes listed by name |
| `session.include_experimental` | list experimental commands (`plugins.experimental_apis`); false when left out |

Every other line is one exchange:

| Key | Meaning |
|---|---|
| `note` | what the exchange shows, for people |
| `client` | the message the client sends, as JSON |
| `client_text` | or the exact text it sends, for lines that aren't valid messages |
| `execute` | when present, the step must be `ExecuteTool` naming this `command_id` and `arguments_json`; when absent, the step must not be one |
| `outcome` | what the daemon hands back after running that command: `{ok, result}`, `{ok, job_id}`, or `{ok: false, diagnostics: [{code, values}]}` (the catalog fills in each diagnostic) |
| `server` | the exact reply; `null` for no reply (a notification) |

`<app_version>` in an expected reply stands for `rastai_core::versions::APP_VERSION`, which every release bumps.

## The registry

`commands.json` holds the commands every transcript's tools are built from (`tools_for_principal`). The ids, scopes
and kinds are real rows of contracts 6.4; the titles and schemas are stand-ins until the core table and its argument
types land (M1-22). `system.env.list` is marked experimental here only to show that such a command is hidden.

## The transcripts

| File | What it shows |
|---|---|
| `initialize.jsonl` | `initialize` keeps the client's revision; `notifications/initialized` gets no reply; `ping`; a second `initialize` is refused |
| `initialize_unknown_revision.jsonl` | a revision the server doesn't speak gets its newest |
| `tools_list_mcp_default.jsonl` | an `mcp-default` token sees only the `read` commands, with their schemas and `readOnlyHint` |
| `tool_call.jsonl` | a call is handed to the daemon as `ExecuteTool`; a result comes back as `structuredContent`, a failure as `isError` with its code |
| `tool_call_job.jsonl` | a job command returns its `job_id` at once, with a note to poll `job.get` |
| `tool_call_outside_scopes.jsonl` | tools the token's scopes don't list are refused (`-32602`, `RRS-C0001`) and never executed |
| `malformed.jsonl` | lines that aren't valid messages, and requests the session can't take, get `RRS-C3001` |
