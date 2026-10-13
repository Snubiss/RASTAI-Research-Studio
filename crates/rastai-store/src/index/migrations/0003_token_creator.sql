-- Who made each token, so revoking a token or disabling a plugin also revokes what it made (contracts 8.2 "Token
-- creators and revocation", threat model R28). The text is the journal's principal: owner, token:<token_id> or
-- plugin:<plugin_id>; empty when nothing recorded the creator.
ALTER TABLE tokens ADD COLUMN created_by TEXT NOT NULL DEFAULT '';

-- A token made before this migration gets its creator from the journal: the principal of the successful token.create
-- row whose result names it. The result keeps token_id (it isn't secret) and redacts only the token's text. The CASE
-- keeps json_extract off a result that isn't JSON, which would stop the migration. The oldest row wins, though a
-- token id is only ever made once.
UPDATE tokens SET created_by = COALESCE(
    (
        SELECT command_journal.principal
        FROM command_journal
        WHERE command_journal.command_id = 'token.create'
            AND command_journal.ok = 1
            AND CASE
                WHEN json_valid(command_journal.result_json) THEN json_extract(command_journal.result_json, '$.token_id')
            END = tokens.token_id
        ORDER BY command_journal.started_unix_ms, command_journal.journal_id
        LIMIT 1
    ),
    ''
);

-- A successor made before this rule must not outlive its creator: every live token whose filled-in chain leads to a
-- revoked token, or to a plugin that is disabled or uninstalled, is revoked now (in Unix milliseconds). The chain is
-- followed through tokens already revoked, and never through the owner, who is never revoked.
UPDATE tokens SET revoked_unix_ms = CAST(unixepoch('subsec') * 1000 AS INTEGER)
WHERE revoked_unix_ms IS NULL
    AND token_id IN (
        WITH RECURSIVE
            dead_creators (creator) AS (
                SELECT 'token:' || token_id FROM tokens WHERE revoked_unix_ms IS NOT NULL
                UNION
                SELECT 'plugin:' || plugin_id FROM plugins WHERE state IN ('disabled', 'uninstalled')
            ),
            swept (token_id) AS (
                SELECT token_id FROM tokens WHERE created_by IN (SELECT creator FROM dead_creators)
                UNION
                SELECT tokens.token_id FROM tokens JOIN swept ON tokens.created_by = 'token:' || swept.token_id
            )
        SELECT token_id FROM swept
    );
