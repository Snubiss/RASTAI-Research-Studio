# Three-surfaces MCP transcripts

Each `.jsonl` file is one MCP session through the real `rastai mcp` stdio bridge against a running daemon, as
`tests/e2e/three_surfaces/` records it (plan M1-49; flows/platform.md 4.3). The same test runs the same commands
over Connect JSON on HTTP and as `rastai` verbs, and checks that all three agree; these files pin what MCP shows.
Where `fixtures/mcp/server/` replays `rastai-mcp` alone with made-up outcomes, these come from the whole path:
bridge, `/mcp`, `execute_command`, the handlers and a real TextCNN train job.

## Format

The first line names the session's token: `session.token_scopes` is the scopes `token.create` gave it, which the
test checks.

Every other line is one message the test sent, in order:

| Key | Meaning |
|---|---|
| `note` | what the exchange shows, for people; the test ignores it |
| `client` | the message the test sent |
| `server` | the reply, exactly; `null` for a notification |

Values that change from run to run are written as placeholders, in what was sent and in what came back, inside
text items that hold JSON too:

| Placeholder | Stands for |
|---|---|
| `<job_id>`, `<run_id>`, `<token_id>`, `<node_id>` | any id with that prefix (`job_`, `run_`, `tok_`, `node_`) |
| `<repo>` | the repository's folder (the design and dataset paths) |
| `<app_version>` | `app_version`, and the server's version in `initialize` |
| `<unix_ms>` | a node's `last_heartbeat_unix_ms` |

When a transcript stops matching, the test prints the lines its run recorded, in this format without the notes.
A change to what MCP shows updates the file in the same PR, with its notes.

## The transcripts

| File | Token | What it shows |
|---|---|---|
| `three_surfaces.jsonl` | `read` and `training` (an agent allowed to train) | `system.status`; `train.start` answering its job id at once; `job.get` of an unknown job (`RRS-J0005`); an uninstalled recipe (`RRS-T0002`); a missing argument (`RRS-C0002` with `RRS-E1001`); after the jobs end, `job.get` of the jobs HTTP, the CLI and this session started (succeeded) and of one that failed (`RRS-D3001`) |
| `chat_only_token.jsonl` | the `chat-only` preset | no tool listed; `system.status`, `train.start` and `job.get` refused with `RRS-C1005` |
