-- rastai.db, migration 1: every table of contracts 8.2, in its order and with its columns, except archive_cursors,
-- which M11's migration adds (empty, so it moves no live data). There is no settings table: settings live only in
-- the files of contracts 11.1.
--
-- Conventions, so the row structs in index/ stay plain:
-- - STRICT tables: SQLite refuses a value of the wrong type instead of storing it as written.
-- - Times are Unix milliseconds in INTEGER columns named *_unix_ms; NULL means "not yet" (finished, revoked).
-- - Flags are INTEGER 0 or 1. Lists of words (scopes, capabilities, permissions) are JSON arrays of text.
-- - An id that a row may lack (a run's parent, a document's project) is '' rather than NULL.
-- - No CHECK lists of allowed values: SQLite can't change a CHECK without rebuilding the table, and these lists
--   grow (lineage relations, states). The writers check values; contracts 8.2 and 9 list them.
-- - Foreign keys only where a child row means nothing without its parent and is deleted with it.
-- The migration runs inside a transaction with its schema_migrations row (sqlite.rs), so it holds no BEGIN.

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_unix_ms INTEGER NOT NULL
) STRICT;

-- A project's data policy is not here: it is a setting, read from the project's settings.json (contracts 11.1).
CREATE TABLE projects (
    project_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    opened_unix_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE documents (
    document_id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    path TEXT NOT NULL,
    project_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_unix_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX documents_by_project ON documents (project_id);

-- One row per step of a document's undo history; position counts from 1 in the order the edits were made.
CREATE TABLE undo_steps (
    document_id TEXT NOT NULL REFERENCES documents (document_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    forward_patch_json TEXT NOT NULL,
    inverse_patch_json TEXT NOT NULL,
    journal_id TEXT NOT NULL,
    PRIMARY KEY (document_id, position)
) STRICT;

-- ok, code, result_json, job_id and finished_unix_ms stay NULL until the command finishes. arguments_json has its
-- secret and content fields redacted before the row is written (contracts 5.1).
CREATE TABLE command_journal (
    journal_id TEXT PRIMARY KEY,
    command_id TEXT NOT NULL,
    principal TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    ok INTEGER,
    code TEXT,
    idempotency_key TEXT,
    result_json TEXT,
    job_id TEXT,
    started_unix_ms INTEGER NOT NULL,
    finished_unix_ms INTEGER
) STRICT;

-- Not unique: a key is honoured for 24 hours (flows/platform.md 3.2), after which the same key may come again.
CREATE INDEX command_journal_by_idempotency_key ON command_journal (principal, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Small, so the startup sweep for commands a restart interrupted (RRS-C0005) never scans the whole journal.
CREATE INDEX command_journal_unfinished ON command_journal (journal_id) WHERE finished_unix_ms IS NULL;

-- token_hash is the BLAKE3 of the token; the token itself is never stored.
CREATE TABLE tokens (
    token_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    created_unix_ms INTEGER NOT NULL,
    revoked_unix_ms INTEGER
) STRICT;

CREATE TABLE nodes (
    node_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    transport TEXT NOT NULL,
    address TEXT NOT NULL,
    state TEXT NOT NULL,
    protocol_version INTEGER NOT NULL,
    capabilities TEXT NOT NULL,
    last_heartbeat_unix_ms INTEGER
) STRICT;

-- acked_sequence is the last event-log record indexed for the run. archive_state is local until the archive (M11)
-- marks the run archived or pruned (contracts 9.9); nothing in M1 changes it.
CREATE TABLE runs (
    run_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    project_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    parent_run_id TEXT NOT NULL,
    record_hash TEXT NOT NULL,
    acked_sequence INTEGER NOT NULL DEFAULT 0,
    created_unix_ms INTEGER NOT NULL,
    finished_unix_ms INTEGER,
    pinned INTEGER NOT NULL DEFAULT 0,
    archive_state TEXT NOT NULL DEFAULT 'local'
) STRICT;

CREATE INDEX runs_by_project ON runs (project_id, created_unix_ms);

-- Keyed by (run_id, sequence, name), so a re-sent record never counts twice. value is NULL for a NaN: SQLite stores
-- a NaN as NULL, and a diverging run must still log its metrics.
CREATE TABLE run_metrics (
    run_id TEXT NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    step INTEGER NOT NULL,
    name TEXT NOT NULL,
    value REAL,
    emitted_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (run_id, sequence, name)
) STRICT;

CREATE INDEX run_metrics_by_name ON run_metrics (run_id, name, sequence);

CREATE TABLE run_tags (
    run_id TEXT NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (run_id, tag)
) STRICT;

-- A job may have no run (a serve job), so run_id is '' then and has no foreign key. pid, process_group_id and
-- exit_code are NULL until known.
CREATE TABLE jobs (
    job_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    pid INTEGER,
    process_group_id INTEGER,
    exit_code INTEGER,
    error_code TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX jobs_by_run ON jobs (run_id);

CREATE TABLE checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    step INTEGER NOT NULL,
    folder TEXT NOT NULL,
    node_id TEXT NOT NULL,
    is_best INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX checkpoints_by_run ON checkpoints (run_id, step);

CREATE TABLE artifacts (
    artifact_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    home_node_id TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_unix_ms INTEGER NOT NULL
) STRICT;

-- The blob GC builds its live set from content hashes.
CREATE INDEX artifacts_by_content_hash ON artifacts (content_hash);

CREATE TABLE artifact_copies (
    artifact_id TEXT NOT NULL REFERENCES artifacts (artifact_id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    path TEXT NOT NULL,
    verified_unix_ms INTEGER,
    PRIMARY KEY (artifact_id, node_id)
) STRICT;

-- from_id is the input and to_id what was made from it (contracts 8.2 lists the relations).
CREATE TABLE lineage (
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    PRIMARY KEY (from_id, to_id, relation)
) STRICT;

CREATE INDEX lineage_by_to ON lineage (to_id);

-- model_hash is '' until known: models found only by scanning the HF cache get it lazily from model.inspect.
CREATE TABLE models (
    model_ref TEXT PRIMARY KEY,
    model_hash TEXT NOT NULL DEFAULT '',
    family TEXT NOT NULL DEFAULT '',
    license TEXT NOT NULL DEFAULT '',
    gated INTEGER NOT NULL DEFAULT 0,
    twin_of TEXT NOT NULL DEFAULT '',
    twin_same_weights INTEGER NOT NULL DEFAULT 0,
    format_info_json TEXT NOT NULL DEFAULT '{}',
    size_bytes INTEGER
) STRICT;

CREATE TABLE plugins (
    plugin_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    state TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    granted_permissions TEXT NOT NULL,
    source TEXT NOT NULL
) STRICT;

CREATE TABLE transfers (
    transfer_id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL,
    direction TEXT NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    bytes_done INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL,
    state TEXT NOT NULL
) STRICT;

-- The daemon log's ack, like runs.acked_sequence for a run log. One row per log, keyed by its name.
CREATE TABLE log_cursors (
    log_name TEXT PRIMARY KEY,
    acked_sequence INTEGER NOT NULL
) STRICT;

-- delivered_unix_ms, journal_id and code stay NULL until the handler's command ran.
CREATE TABLE plugin_event_deliveries (
    plugin_id TEXT NOT NULL,
    handler_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    log_name TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    run_id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    queued_unix_ms INTEGER NOT NULL,
    delivered_unix_ms INTEGER,
    journal_id TEXT,
    code TEXT,
    PRIMARY KEY (plugin_id, handler_id, log_name, sequence)
) STRICT;

-- Pending calls per plugin, in log order.
CREATE INDEX plugin_event_deliveries_pending ON plugin_event_deliveries (plugin_id, log_name, sequence)
    WHERE delivered_unix_ms IS NULL;
