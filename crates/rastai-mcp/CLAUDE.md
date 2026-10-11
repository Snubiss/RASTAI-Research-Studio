# rastai-mcp

The in-house MCP: JSON-RPC 2.0 messages, the server side that turns registry commands into MCP tools, the stdio
bridge behind `rastai mcp`, and (M7-E02) the client for harness tools. Its design is
`docs/architecture/layout/rust-control-plane.md` section 7; its names are in `docs/architecture/contracts.md` §2.1
and §15.1, its codes in §10.3 (`RRS-C3001`, `RRS-C3002`).

## Local rules

- **No command execution.** `handle_message` returns a plain `McpStep`; a tool call is `ExecuteTool`, and
  `rastai-server` runs it through `execute_command` (gate, scope, arguments, journal). Nothing here calls back into
  the daemon.
- **No policy decisions.** Which tools a session sees is `check_scope` from `rastai-core` (`tools_for_principal`),
  the one scope rule; network and sandbox policy belong to the node. A tool call isn't checked against that list:
  `execute_command` decides (`RRS-C0001`, `RRS-C1005`), so a missing scope is never reported as an unknown command.
- **No axum and no HTTP serving.** `rastai-server` owns the `/mcp` route; the bridge is an HTTP client through
  `rastai-transport`.
- **Never reads tokens or secrets.** The bridge gets its token as an argument (`rastai-cli` reads `RASTAI_TOKEN`).
  Without one it answers `RRS-C3002` and never contacts the daemon; it never falls back to the owner.
- **Untrusted input is strict.** Every message goes through `parse_message`: over `MAX_MESSAGE_BYTES` refused before
  parsing, the strict JSON reader of `rastai-formats`, no batches, no unknown members. The `jsonrpc_decode` fuzz
  target in `fuzz/` (its own nightly workspace, contracts §12.5) covers it; a new parser here adds its target in the
  same PR (ADR-0037).
- **One error enum**, `McpError` in `src/error.rs`, each variant with a catalog code from contracts §10.3, and one
  constructor for a malformed message, `malformed_message`. Every error reply carries its diagnostic in the JSON-RPC
  error's `data`.
- **`src/lib.rs` and `src/server/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the
  bottom of the file they test; the transcripts (`fixtures/mcp/server/`) and the bridge tests are in `tests/`.
