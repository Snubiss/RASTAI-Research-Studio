# ADR-0003: SQLite index, a separate vault, a BLAKE3 blob store, and the HF cache in place

**Status:** Accepted, 2026-10-10

## Context

- One user per install for now. Team mode comes later; the principal on every command (ADR-0005) keeps it possible.
- Rastai needs queryable state (runs, metrics, lineage, the command journal, settings), sensitive state (conversations, gateway logs), large files (checkpoints, replays, dataset snapshots) and model weights.
- Conversations need encryption at rest in Phase 2 (§0.7) without a migration.
- Model weights are often several GB each. Users already have them in the Hugging Face cache.
- Spike 07: SQLite logging behind a shared mutex convoyed at 256 clients. Moving all writes to one writer thread fixed it. The Rust gateway then held 17,724 req/s with a 28 ms p99 at 256 clients while logging every request.

## Decision

1. **`<data>/rastai.db`**: SQLite in WAL mode through `rusqlite`. It holds the index tables in contracts §8.2 and can be rebuilt from event logs (ADR-0002).
2. **`<data>/vault.db`**: SQLite in WAL mode. It holds `conversations`, `messages` (a tree through `parent_id`), `annotations` and `gateway_log`.
   - Plain SQLite in the prototype. Phase 2 switches it to SQLCipher with the same schema.
   - Incognito conversations never reach either database.
   - Retention deletes `gateway_log` rows by age (`gateway.retention_days`, default 30).
3. **One writer thread per database file**, owned by `rastai-store`. Callers send a record over a channel and await the commit. Readers open their own connections.
4. **Blob store** at `<data>/blobs/<first 2 hex>/<64 hex>`, named by BLAKE3.
   - Writes land in `.incoming/<random>`, are fsynced, then renamed into place, so a blob is either whole or absent.
   - Pinning or exporting a checkpoint registers its files as blobs, as hard links when they're on the same filesystem.
5. **Model weights stay in the HF cache**, never copied. The node reads `$HF_HUB_CACHE`, else `$HF_HOME/hub`, else `~/.cache/huggingface/hub`. Downloads write the standard layout so other HF tools see them. `HF_HUB_OFFLINE=1` is honored.
6. **No Postgres** and no database server.
7. **Disk safety:**
   - quotas and garbage collection with a `pinned` flag (`storage.gc`);
   - checkpoints keep the last 3, the best and any pinned;
   - free space is checked before downloads and training (`storage.free_space_margin_bytes`), failing with an `RRS-S3xxx` code.
8. **Secrets are never stored in either database.** They live in the OS keychain (contracts §8.5).

**Checked by:** a store test with 256 concurrent writers and no lock convoy (one writer thread); a test that incognito chats leave zero rows in `vault.db`; a blob-store kill -9 test that leaves no partial blob under its final name; a test that a model load reads from the HF cache with no copy in `<data>`.

## Consequences

**Good**
- Embedded, zero-ops, one file each, easy to back up.
- Encryption at rest is a later switch on one file, not a migration.
- Blobs are deduplicated and verifiable by name. Weights take no extra disk.
- The one-writer pattern is already measured (spike 07).

**Costs**
- Two databases mean no cross-database transactions. Code that touches both (for example, starring a conversation into a dataset) writes the vault row first and the index row second. The index is rebuildable, so this order is safe.
- SQLite caps write throughput at one writer per file. That's fine for one user. Team mode in Phase 3 may need more.
- Rastai depends on the HF cache layout staying stable. The cache reader is one module, so a change touches one file.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One database for everything | Encrypting conversations later would mean a migration, and sensitive rows would sit with everything else. |
| Postgres | A server process to install and run for one user. |
| An embedded key-value store (sled, RocksDB) | Not on the allowlist. The runs table needs SQL queries. |
| A connection pool with shared writes | Measured convoy at high concurrency (spike 07). |
| Copying weights into the blob store | Doubles disk use for multi-GB files, and other tools can't share them. |

## Links

- Spec: [§19](../spec/rastai_spec_v5.md#19-storage-and-durability-new), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy), [§0](../spec/rastai_spec_v5.md#0-decisions) item 7
- Contracts: [§8 storage](../architecture/contracts.md#8-storage)
- Spike: [07 backend bake-off](../../spike/results/07_backend_bakeoff.md)
- Related: [ADR-0002](0002-durability-worker-event-logs.md), [ADR-0007](0007-local-api-security.md), [ADR-0008](0008-gateway-owns-tokenization.md)
