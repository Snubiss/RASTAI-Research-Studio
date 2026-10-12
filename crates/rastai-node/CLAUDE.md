# rastai-node

The node agent. It runs embedded in the daemon (its own tokio runtime, serving `node.sock`) and as
`rastai node serve`. Its design is `docs/architecture/layout/rust-node-gateway-proto.md` section 7; its names are in
`docs/architecture/contracts.md` §2.1 and §15.1, its codes in §10.3 (the map is layout section 11.1).

## Local rules

- **`node.rs` is the entry.** `run_node` is the start-up order of layout 7.3 as a list of calls; a later step joins
  that list where its comment says, never somewhere else.
- **One Ed25519 wrapper.** Only `identity.rs` imports ed25519-dalek (contracts §2.5, `check-imports`); the daemon
  signs and verifies through it too. A key file is never replaced: a new key is a new node id.
- **Read-only hardware tools.** `nvidia-smi` and `rocm-smi` run through `run_tool` with a timeout, and their output
  is parsed, never trusted. A failure is a note (`RRS-H0002`), never an error that stops the node. Nothing here sets
  `HSA_OVERRIDE_GFX_VERSION` or picks an environment: that is `environments.rs` (M2-16), which never forces ROCm on
  an unsupported chip.
- **Locks are short.** Each table in `NodeState` has its own `std::sync::Mutex`, taken by a small function that
  copies or changes and lets go; never across an `.await`.
- **The edge turns errors into diagnostics.** Library functions return `NodeError`; the tonic service puts a refusal
  in the response as a `Diagnostic` with gRPC status OK (contracts 4).
- **A job's end is a state, not an error.** `start_job` returns the job's `JobInfo` even when it refuses or fails the
  start (its diagnostic is `last_error`), and `signal_job` returns the `RRS-J0004` diagnostic of a signal it didn't
  deliver. Inside `jobs/`, a step that fails says why in a `String`, which becomes that diagnostic's `reason`.
- **One live job per run.** `admit_job` checks for a live job of the run and reserves the run under one hold of
  `NodeState.jobs`; `job.json` is written only after that, and the run is given back only after the terminal
  `job.json` is written (contracts 9.1). A group signal goes through the `kill` tool, and only after the pid's boot
  id, start ticks and process group were checked again (`signals.rs`).
- **Reattach before worker-host.sock.** `reattach_jobs` enters every live job from its `job.json` before the socket a
  worker re-registers on exists. A `job.json` names a live process only when the pid leads its own group and still
  runs with the recorded boot id and start ticks (`recorded_process`, `is_running_process`); anything else is gone
  and is never signalled. A gone job whose `stderr.log` ends with `RRS-J3006` takes nothing from the run's log.
- **State changes ask for a heartbeat.** Whoever changes a job's state calls `request_heartbeat` (it only wakes the
  loop); the daemon learns job states from heartbeats alone.
- **Unix first, Windows compiles.** Socket and file-mode code sits behind `#[cfg(unix)]`; `cargo clippy --target
  x86_64-pc-windows-gnu -p rastai-node` must stay clean. The integration tests are Unix only.
- **Fixtures are recordings.** `fixtures/hardware/` holds what real machines printed. A new case says in its README
  row what was recorded and what was cut or changed.
- **`src/lib.rs` is a list of `pub mod` lines.** Unit tests sit at the bottom of the file they test; the socket and
  fixture tests are in `tests/`, with their shared helpers in `tests/common/mod.rs`.
