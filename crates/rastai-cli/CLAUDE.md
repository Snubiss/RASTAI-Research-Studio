# rastai-cli

The `rastai` binary: process modes, the in-house argument parser, the verbs generated from the command registry,
following jobs, `--json` output and the exit codes. Its design is `docs/architecture/layout/rust-control-plane.md`
section 9; its names are in `docs/architecture/contracts.md` §2.1 and §15.1.

## Local rules

- **No business logic.** Every verb is a registry command the daemon runs; the CLI never keeps a list of verbs. The
  fixed words (`CLI_MODE_WORDS`, `ALIASES`, `GLOBAL_FLAGS`) come from `rastai-core`'s naming module, and a command's
  flags come from its `arguments_schema_json`.
- **Only `output.rs` prints.** Text mode: results on stdout, diagnostics on stderr. `--json`: one JSON object per
  line on stdout, ending with `{"exit_code": n, "diagnostics": [...]}`. `rastai mcp`'s stdout is the MCP channel.
- **Local commands are only those that must work without a daemon:** `validate`, `version`, `help`, and (M1-33)
  `storage rebuild-index`. Help never starts a daemon; it falls back to the command cache.
- **daemon.sock only, as the owner by peer credentials.** The CLI never reads `owner.token` and never opens
  `rastai.db` (except, from M1-33, inside the offline rebuild with `daemon.lock` held).
- **Exit codes are a contract** (layout 9.5, `exit_codes.rs`). Every refusal is a catalog diagnostic, and
  `CliError`'s variant picks the code.
- **A daemon the CLI starts runs on its own:** `rastai serve --headless --json` in its own process group. Tests
  that start one stop it on every exit path (`tests/common`'s `DaemonGuard`).
- **`src/lib.rs` is a list of `pub mod` lines** (contracts §12.2); `main.rs` is the thin binary. Unit tests sit at
  the bottom of the file they test; the parser table, the copy-as golden, the exit codes against a fake daemon and
  the local commands are in `tests/`.
