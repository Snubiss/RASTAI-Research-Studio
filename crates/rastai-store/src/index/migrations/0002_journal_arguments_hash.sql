-- The idempotency check compares this, not the redacted arguments_json: two requests that differ only in a secret
-- field redact to the same text. It is a keyed BLAKE3 of the canonical, unredacted arguments (rastai-server
-- journal.rs), written only for a row with an idempotency key; NULL otherwise and for rows a rebuild makes.
ALTER TABLE command_journal ADD COLUMN arguments_hash TEXT;
