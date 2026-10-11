# rastai-transport

The byte pipes of the backend: local sockets, gRPC channels over them, the HTTP client, and later SSH, server-sent
events and resumable transfers. Its design is `docs/architecture/layout/rust-node-gateway-proto.md` section 4; its
names are in `docs/architecture/contracts.md` §2.1 and §15.1, its codes in §10.3 (`RRS-N2004` to `RRS-N2009`).

## Local rules

- **Decides nothing about what flows through the pipes.** No database, no settings files, no retries: callers pass
  paths and options in, and decide what to do with an error.
- **Every path comes from `rastai-core`'s `paths.rs`.** This crate never builds a path itself.
- **Local sockets are same-user only.** `bind_local_socket` creates the socket folder 0700 and refuses one another
  user owns, group or others can use, or that is not a folder (a symbolic link, even a dangling one, or a file);
  clients check the same folder before they connect. `accept_local` and
  `connect_local_socket` also check the peer's user (`peer.rs`). The gRPC client (`connect_grpc_channel`) relies on
  the folder check alone, because tonic's `unix:` endpoint reads no peer credentials; the gRPC server accepts
  through `accept_local`. There is no TCP bind function here: the daemon's TCP listener is `rastai-server`'s.
- **One wrapper file per library** (contracts §2.5, `check-imports`): only `http_client.rs` imports hyper and
  hyper-util (and rustls from M2-14); only `grpc_channel.rs` would import tower. Nothing imports blake3.
- **No `unsafe` and no libc.** Our own user id comes from a socket pair (`peer.rs`, whose header says why
  `rastai-core` reads it from /proc/self instead), not getuid.
- **Unix first, Windows compiles.** Unix-only code sits behind `#[cfg(unix)]`, and on Windows each public function
  returns `NotSupportedOnThisPlatform` (`RRS-N2009`) until named pipes arrive (tier 2). The local-socket enums have
  no variant there yet.
- **One error enum**, `TransportError` in `src/error.rs`, each variant with a catalog code from contracts §10.3.
- **`src/lib.rs` is a list of `pub mod` lines** (contracts §12.2). Unit tests sit at the bottom of the file they
  test; the socket, gRPC and HTTP tests are in `tests/`.
