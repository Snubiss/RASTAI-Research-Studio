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
  (`tests/dispatch_complete.rs` fails until it does); until its handler exists the arm answers its area's
  `not_available_code` (every M1 command has its handler, so no arm does now). A handler with side effects makes none on a dry run.
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
- **MCP runs as its bearer token.** `http/mcp_endpoint.rs` takes the principal from `Authorization: Bearer` through
  the gate's `check_bearer`, on daemon.sock too, where the gate alone admits the peer as the owner: the `rastai mcp`
  bridge comes in that way with a scoped token, and MCP never falls back to the owner. Every `tools/call` ends in
  `execute_command`; `rastai-mcp` only turns messages into plain `McpStep`s.
- **Fail closed.** A header the gate reads that is malformed or sent twice is refused, never guessed at. A path with
  `..`, `//`, `%` or `\` is never public. A non-loopback `daemon.listen` is used only as asked, with a warning.
- **Secrets are kept as hashes.** Login codes, session ids, session keys and the owner token are compared by
  `hash_bytes` of their text; the plain values live only in the answer that hands them out.
- **`owner.token` is never rewritten.** A too-open one is `RRS-C1008`, an unusable one `RRS-C1010`; both stop the start
  before any database opens or listener binds (`auth_files.rs`).
- **Startup order is `daemon.rs`'s.** The lock, the owner token, the store, then the listeners last, so
  `/api/health` answers only when everything behind it is ready. `start_daemon` and `wait_for_shutdown` are split so
  tests run a daemon on temporary folders and stop it with `request_shutdown`, without signals.
- **The daemon's durable records go through `record_daemon_event`** (`events/daemon_events.rs`): appended to
  `<data>/daemon/events.log` with an fsync, indexed, then published, under one lock so the cursor follows the log.
  Nothing else writes that log, and nothing publishes a durable record before it is indexed.
- **Run logs are indexed by their tail only** (`indexing/run_tail.rs`): the ack commits with the rows, a remote run's
  mirror is written before the index, and the node is reached through `RunEventSource`, never directly. One tail per
  run (`Server.tailed_run_ids`): the tail re-watches a broken stream itself, and `start_run_tails` skips live tails.
  A `RunEventSource` is one long-lived object per node that survives reconnects, never one per connection.
- **Nodes are reached through `nodes/` only.** `nodes/node_links.rs` and `nodes/node_hub.rs` are the only files that
  import tonic (contracts 2.5): every `NodeService` call is a function there (`start_job_on_node`, ...), each carrying
  the session token, and the hub refuses a call without it (`RRS-N0009`). The embedded node is reached over node.sock
  with the same handshake a remote node gets; it runs on its own thread and runtime (`daemon.rs`).
- **Job states come from the node; one move is recorded once.** Heartbeats and `ListJobs` become `job.state_changed`
  records under `Scheduler.job_state_lock` (`nodes/reconcile.rs`), which is never held across a node call. The launch
  holds `Scheduler.launch_lock` from its queue check to the recorded answer, and a queued job's cancel and a run's
  settle take it first. A run's end is written by the daemon only for a job that ended without one, only
  after the run's tail is over, and only along a contracts 9.5 move (`settle_run`, `daemon_written_run_end`).
- **Queued jobs are rows.** `submit_job` writes `run.json` (with the `JobSpec`), the `runs` row and the `jobs` row
  before it answers; the launch reads the spec back from `run.json`, so a restart loses no queued job.
- **Streams end on the shutdown flag.** `Subscribe` streams and run tails select on `wait_until_stopping`, and a
  stream lets the server go before its last envelope, so a stopping daemon can close its store.
- **One error enum**, `ServerError` in `src/error.rs`; errors of the crates it calls keep their own code.
- **`src/lib.rs` and `src/http/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the bottom
  of the file they test; the gate table and the end-to-end requests are in `tests/gate.rs`, startup and shutdown in
  `tests/startup.rs`, the bus and `Subscribe` in `tests/bus_resume.rs`, run tails and the daemon log's replay in
  `tests/run_tail.rs`, the node link in `tests/node_link.rs` and the real train job (`#[ignore]`d, `just
  test-daemon-jobs`) in `tests/train_jobs.rs`, sharing `tests/common/mod.rs`. Every test removes its temporary folder at its end.
