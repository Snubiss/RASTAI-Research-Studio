# rastai-server

The daemon, and the composition root: the only crate that wires the others together. Its design is
`docs/architecture/layout/rust-control-plane.md` section 8; the request gate is section 8.3a, with
`docs/architecture/flows/platform.md` 2.1 (startup) and 4.1–4.2 (sign-in and the gate) and ADR-0034. Its names are in
`docs/architecture/contracts.md` §2.1 and §15.1, the HTTP surface in §3.3, its codes in §10.3.

## Local rules

- **Handlers never see HTTP.** Only files under `src/http/` import axum (contracts §2.5), and only `http/connect.rs`
  imports prost. Command handlers take `(&CommandContext, Args)` and return `Result<CommandOutput, Vec<Diagnostic>>`.
- **Every caller ends in `execute_command`** (`commands/execute.rs`, flows/platform.md 3.2): scope before arguments,
  arguments through `check_arguments`, then the journal. A new core command adds one arm to `commands/dispatch.rs`
  (`tests/dispatch_complete.rs` fails until it does); until its handler exists the arm is `not_available_yet`, its
  area's code. A handler with side effects makes none on a dry run.
- **The journal never holds a secret.** `secret` and `content` fields are redacted in `arguments_json` and
  `result_json` alike (`commands/journal.rs`); what can't be judged is redacted whole. An idempotent replay returns
  the stored, redacted result.
- **The gate runs first, on every path.** `http/router.rs` puts `check_request_gate` in front of every route and the
  fallback; a new route is added inside `build_router`, never beside it. The gate is one plain function over a
  `RequestHead`; header parsing is `read_request_head`, and the `request_gate` fuzz target covers both (ADR-0037).
- **Connect JSON is protobuf-es's.** `http/connect_json.rs` writes, accepts and refuses exactly what protobuf-es does
  (layout 11.3); a change to it starts with a case in `fixtures/proto-golden/commands/connect_json/cases.json`, which
  `just test-js` checks against protobuf-es and `tests/connect_protocol.rs` against the codec. Its decoders read
  untrusted bodies, so the `connect_json` fuzz target covers them (ADR-0037).
- **Fail closed.** A header the gate reads that is malformed or sent twice is refused, never guessed at. A path with
  `..`, `//`, `%` or `\` is never public. A non-loopback `daemon.listen` is used only as asked, with a warning.
- **Secrets are kept as hashes.** Login codes, session ids, session keys and the owner token are compared by
  `hash_bytes` of their text; the plain values live only in the answer that hands them out.
- **`owner.token` is never rewritten.** A too-open one is `RRS-C1008`, an unusable one `RRS-C1010`; both stop the start
  before any database opens or listener binds (`auth_files.rs`).
- **Startup order is `daemon.rs`'s.** The lock, the owner token, the store, then the listeners last, so
  `/api/health` answers only when everything behind it is ready. `start_daemon` and `wait_for_shutdown` are split so
  tests run a daemon on temporary folders and stop it with `request_shutdown`, without signals.
- **One error enum**, `ServerError` in `src/error.rs`; errors of the crates it calls keep their own code.
- **`src/lib.rs` and `src/http/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the bottom
  of the file they test; the gate table and the end-to-end requests are in `tests/gate.rs`, startup and shutdown in
  `tests/startup.rs`, sharing `tests/common/mod.rs`. Every test removes its temporary folder at its end.
