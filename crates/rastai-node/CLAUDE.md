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
- **Unix first, Windows compiles.** Socket and file-mode code sits behind `#[cfg(unix)]`; `cargo clippy --target
  x86_64-pc-windows-gnu -p rastai-node` must stay clean. The integration tests are Unix only.
- **Fixtures are recordings.** `fixtures/hardware/` holds what real machines printed. A new case says in its README
  row what was recorded and what was cut or changed.
- **`src/lib.rs` is a list of `pub mod` lines.** Unit tests sit at the bottom of the file they test; the socket and
  fixture tests are in `tests/`, with their shared helpers in `tests/common/mod.rs`.
