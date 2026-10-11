# rastai-store

Everything the daemon persists. Its design is `docs/architecture/layout/rust-control-plane.md` section 6; its names
are in `docs/architecture/contracts.md` §8.2 (tables and columns) and §15.1 (code names).

## Local rules

- **The only SQLite user.** No other crate opens `rastai.db` or `vault.db`, and `src/sqlite.rs` is the only file
  that imports rusqlite. The SQL text lives with its table in `src/index/`; callers pass row structs and filters,
  never SQL, so the functions that run SQL are `pub(crate)`.
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
  bottom of the file they test; the writer and migration tests are in `tests/`.
