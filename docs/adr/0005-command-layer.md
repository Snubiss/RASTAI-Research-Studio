# ADR-0005: One generic Execute endpoint over a command registry

**Status:** Accepted, 2026-10-10

## Context

- Every action must be reachable from the UI, the command palette, keybindings, menus, CLI verbs, MCP tools and "Copy as CLI / Python / MCP" (§7, §21).
- If each surface got its own endpoint, each new action would touch proto, the TS client, the CLI parser and the MCP server by hand, and those four would drift.
- Plugins add commands at runtime, so the set isn't known at compile time.
- Team mode (Phase 3) needs to know who did what, so every call needs a principal from day one.
- Undo is expected in editors, but side effects (a training run, a download) can't be undone honestly.

## Decision

1. **Every action is a command** with:
   - an id: `<area>.<verb>` or `<area>.<noun>.<verb>`;
   - typed arguments and result (`<Name>Args` / `<Name>Result` in `rastai-formats/src/commands/`);
   - a permission scope (contracts §6.3);
   - a kind: `sync` (target under 2 s), `job` (returns a `job_id` at once) or `stream`;
   - an `undoable` flag.
2. **One generic RPC:** `CommandService.Execute(command_id, arguments_json, project_id, idempotency_key, dry_run)`, plus `ListCommands`. Both are served by the daemon.
3. **The registry is a plain table plus a `match`.**
   - `COMMANDS: &[CommandDescriptor]` lives in `rastai-core`;
   - `dispatch()` in `rastai-server` is one `match` on the id;
   - unknown ids fall through to the plugin host, which knows plugin commands from manifests.
   - No macros, no reflection (contracts §6.2).
4. **The principal is never in the request.** The daemon derives it from the session cookie, bearer token, socket peer or plugin job token. It is written to `command_journal` with the arguments and the outcome code.
5. **Generated surfaces:**
   - CLI verb = the id's segments as words (`rastai eval suite edit`);
   - MCP tool = dots replaced with `__` (`eval__suite__edit`);
   - palette entry = `title`.
   - The in-house MCP server and client (`rastai-mcp`, JSON-RPC 2.0) are generated from the same registry.
6. **Undo covers document edits only.** An undoable command records a forward and an inverse JSON Patch in `undo_steps`, and `document.undo` / `document.redo` replay them. Runs, downloads, installs and deletions are not undoable; the UI says so before it acts.
7. **Errors:** a failed command returns `ok = false` with one or more `Diagnostic`s carrying catalog codes (`RRS-C` for dispatch and auth; the owning area's prefix otherwise).

```rust
// crates/rastai-server/src/commands/dispatch.rs
// Failures are always catalog diagnostics, so there is no separate CommandError type (contracts §6.2).
pub fn dispatch(context: &CommandContext, command_id: &str, arguments_json: &str) -> Result<CommandOutput, Vec<Diagnostic>> {
    match command_id {
        "model.load" => model_commands::load(context, parse_arguments(command_id, arguments_json)?),
        "chat.send" => chat_commands::send(context, parse_arguments(command_id, arguments_json)?),
        _ => plugin_host::forward_command(context, command_id, arguments_json),
    }
}
```

*Note since acceptance (2026-10-10): the example now matches contracts §6.2 (handlers return `Vec<Diagnostic>`; `parse_arguments` takes the command id to find the argument description it checks the JSON against, control-plane layout §4). The decision is unchanged.*

**Checked by:**
- a test that every `COMMANDS` row has a `match` arm, argument and result types, and a CLI and MCP name;
- a test that runs every command with `dry_run` and expects no crash and no un-coded error;
- one test per scope that a token without it gets `RRS-C1xxx`;
- snapshot tests of the generated CLI help and MCP tool list.

## Consequences

**Good**
- Adding a command is one table row, one match arm and one handler. Every surface gets it for free.
- Plugin commands and core commands work identically for the UI, CLI and MCP.
- The journal records every action with its principal, which is ready for audit logs and team mode.

**Costs**
- Arguments travel as JSON strings, so proto tooling can't type-check them. `rastai-formats` validates them at the daemon instead.
- The table and the `match` must be kept in step by hand. The first test above enforces this.
- Users can't undo a side effect. Destructive commands need a confirmation step in the UI.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One RPC or REST route per action | Four surfaces to update per action, and they drift. Plugins can't add routes. |
| GraphQL | A schema layer, and a library, for what is a flat list of actions. |
| Undo for side effects via compensating actions | "Undo training" would mean deleting a run. That's confusing and loses data. |
| An external MCP SDK | Not on the allowlist. The JSON-RPC surface is small and generated from our registry anyway. |

## Links

- Spec: [§7](../spec/rastai_spec_v5.md#7-ux-system), [§13](../spec/rastai_spec_v5.md#13-plugins-and-ecosystem), [§21](../spec/rastai_spec_v5.md#21-command-layer-and-local-api-security)
- Contracts: [§4.2 CommandService](../architecture/contracts.md#4-wire-contracts), [§6 commands](../architecture/contracts.md#6-commands)
- Related: [ADR-0004](0004-schema-ownership.md), [ADR-0007](0007-local-api-security.md), [ADR-0030](0030-diagnostics-catalog.md)
