# rastai-store

Everything the daemon persists. Its design is `docs/architecture/layout/rust-control-plane.md` section 6; its names
are in `docs/architecture/contracts.md` §8.2 (tables and columns) and §15.1 (code names).

## Local rules

- **The only SQLite user.** No other crate opens `rastai.db` or `vault.db`, and `src/sqlite.rs` is the only file
  that imports rusqlite. The SQL text lives with its table in `src/index/`; callers pass row structs and filters,
  never SQL, so the functions that run SQL, the migration types and `open_database_for_writing` are `pub(crate)`.
- **Writes only through the writer thread** (`write_index`), one thread per database file, never a
  `Mutex<Connection>` (spike 07). Reads are blocking functions on a pooled read-only connection; async callers run
  them in `spawn_blocking`.
- **Migrations are append-only.** A released `src/index/migrations/<n>_*.sql` is never edited; a schema change is a
  new numbered file in `INDEX_MIGRATIONS`. Every table and column is in contracts §8.2 first or in the same PR
  (`tests/index_migrations.rs` compares the two). There is no `settings` table: settings live only in files.
- **Files before rows on create, rows before files on delete** (layout 6.6): a crash leaves an unreferenced file,
  never a dangling row.
- **No policy here.** Incognito, retention, quotas and what is pinned are decided by `rastai-server` and handed in.
- **One error enum**, `StoreError` in `src/error.rs`, each variant with an `RRS-S` code from contracts §10.3.
- **`unsafe`** only in `src/disk_space.rs` (ADR-0033, M1-25); nowhere else.
- **`src/lib.rs` and `src/index/mod.rs` are lists of `pub mod` lines** (contracts §12.2). Unit tests sit at the
  bottom of the file they test; the writer and migration tests are in `tests/`, sharing `tests/common/mod.rs`. A
  test that needs the crate-private SQL functions (a newer app's migration, a full disk) stays at the bottom of
  `src/`. Every test removes its temporary folder at its end.
- **A failed read is `RRS-S0006`, a failed write `RRS-S3005`** (or `RRS-S3002` for a full disk): `sqlite.rs`
  decides by the connection, a reader's or the writer's. A reader that can't open its file failed a read too;
  `RRS-S0004` ("not writable") is the writer's alone. Event logs follow the same split: opening or reading a log is
  `RRS-S0006`, creating, writing or syncing one `RRS-S3005` (`error.rs`).
- **One indexing path.** Live run logs, the daemon log, the rebuild and restored frames all index through
  `apply_event_record` (`indexer.rs`), whose writes are upserts or updates keyed by ids. `ApplyRunEvents` and
  `ApplyDaemonEvents` skip records at or below the ack and commit the rows with the ack in one transaction, so each
  record is applied exactly once.
- **Blobs are named by their BLAKE3** (`blobs/`): written to `.incoming/`, fsynced, renamed into `ab/<hash>`; a
  mismatch is moved to `.quarantine/` (`RRS-S1001`). A hash is checked for `blake3:<64 lowercase hex>` before it
  becomes a path.
- **One parser of event-log bytes.** Every reader of a log goes through `event_log/reader.rs`, and only
  `event_log/frame.rs` encodes or decodes frame bytes; the `event_frame` fuzz target covers both (ADR-0037).
- **The runtime folder may be shared `/tmp`.** `take_daemon_lock` refuses (`RRS-S0007`) a runtime folder that is a
  link, another user's or open to group or others, and a `daemon.lock` that is a link or not a regular file, before
  writing anything; the lock file is never truncated on open, only rewritten once the lock is held.
