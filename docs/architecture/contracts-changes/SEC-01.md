- **SEC-01 (contract):** Tokens record their creator, so revocation follows what a principal made (threat model R28, issue #230; the owner decided on 2026-10-12 to fix it in M1).
  Before this, a token or plugin holding `system.admin` could make another `system.admin` token, and revoking it left that successor live.
  - **The column.** 8.2's `tokens` gains `created_by` (M1): the principal that ran `token.create`, as `principal_label` writes it in `command_journal.principal` (6.3): `owner`, `token:<token_id>` or `plugin:<plugin_id>`, the owner narrowed by `acting_plugin_id` included.
    8.2 gains "Token creators and revocation".
  - **Revoking cascades.** `token.revoke` revokes, in the same write transaction and with the same `revoked_unix_ms`, every live token whose `created_by` chain leads to the revoked one.
    Once that commits, each is marked revoked in the gate's cache and gets `RRS-C1006`.
    Revoking a token already revoked still sweeps the live tokens under it.
  - **Disabling a plugin cascades.** A plugin that becomes `disabled` (`disable_plugin`, which `plugin.disable` (M8) calls) or `uninstalled` (M8) revokes the tokens whose chain leads to it.
    That happens in the same write transaction that saves its `plugins` row, or in a transaction of its own for a dev plugin, which has no row.
  - **No gap for a racing create.** `token.create` writes its row only while its creator is live:
    a `token:` creator is checked in the writer transaction that inserts the row (`RRS-C1006` when revoked);
    a `plugin:` creator is checked against the plugin host's state under one plugin-host lock, which the create holds until its insert commits and a disable or uninstall holds until its cascade commits and is in the cache (`RRS-C1005` when the plugin isn't `enabled`).
    So a dev plugin is checked like any other, and a create racing a revoke can't leave a successor behind.
  - **The gate's cache.** `token.create` adds its cache entry before it sends its insert and removes it if the insert fails or is refused.
    So a cascade that commits after the insert always finds the entry to mark, and no create puts a live entry back.
  - **The start sweep.** After the plugins load and before the HTTP listener opens, the daemon revokes the live tokens whose chain leads to a plugin the host holds no plugin for, or holds as `disabled`.
    This is how dropping a dev path (owner-only, read at start) revokes that plugin's tokens.
    It also covers an `uninstalled` row, a folder removed by hand and a disable cut short by a crash.
  - **Chains.** The owner is never revoked, so a chain is followed only through `token:` and `plugin:` creators.
    A plugin installed from M8 by a token holding `plugins.manage` chains to `plugin:<plugin_id>`, not to that token; M8-E02 (#136) decides how an install records who asked for it.
  - **Migration `0003`** (`0003_token_creator.sql`, the next free number on `main`) is plain SQL.
    It adds the column as `TEXT NOT NULL DEFAULT ''`.
    It fills existing rows with `json_extract` from the `principal` of the successful `token.create` journal row whose `result_json` names the token (the id isn't `secret`).
    With a recursive common table expression it revokes, at the migration's time, every live token whose filled-in chain leads to a revoked token or a `disabled` or `uninstalled` plugin.
    `crates/rastai-store/tests/index_migrations.rs` tests it.
  - **The command type.** `TokenSummary` (6.5, `crates/rastai-formats/src/commands/token.rs`) gains `created_by`, a required string, so `token.list` shows each token's creator.
    `just codegen` regenerates `schemas/commands/rastai.command.token.list/1.json`, the JSDoc type and the Python dataclass.
    `token.list` is `preview`, so the newly required field passes `check-schema-diff` with this entry.
    Until the code PR lands, the handler answers `created_by` with the empty text, as for a token no record names.
  - **The migration test's reader.** `crates/rastai-store/tests/index_migrations.rs`, which compares 8.2 with the migrated tables, now reads a column's own milestone marker as `check-names` does (12.6: in a list cell a marker covers only the name it follows), so `created_by` (M1) waits for the code PR's migration.
  - 6.4's `token.*` row, flows/platform.md 4.3 and threat model R28 (now owned by SEC-01) say the same.
  - **Choices for the owner's review:**
    - `created_by` uses the journal's principal text rather than a bare id, so a token id and a plugin id can't be confused.
    - `TokenRevokeResult` doesn't list the tokens a cascade revoked (`token.list` with `include_revoked` shows them, each with the same `revoked_unix_ms`).
    - A plugin the host moves to `incompatible` or `failed` keeps its tokens, since that isn't the owner's decision and its own principal holds no scope while it isn't enabled.
    - The migration backfills from the journal and revokes already-orphaned successors rather than leaving pre-existing tokens outside the rule.
    - A plugin's liveness comes from the plugin host under a lock rather than from `plugins.state`, because a dev plugin has no row.
- **SEC-01 (contract, review round 2):** Only the owner may set `ExecuteRequest.acting_plugin_id` (4.2's `ExecuteRequest` row and the paragraph under it, 3.2's "Plugin UIs", 6.3, 8.2's new bullet "Only the owner narrows").
  Before this, `execute_command` narrowed any caller, so a leaked `system.admin` token could send `token.create` with `acting_plugin_id` naming an enabled plugin granted `system.admin`.
  The new token would be recorded as `plugin:<plugin_id>`, and revoking the leaked token wouldn't reach it; a plugin could do the same through another plugin.
  - **The refusal.** `execute_command` refuses a non-empty `acting_plugin_id` from every principal but the owner (the owner's session, `owner.token`, the `daemon.sock` peer) with `RRS-C1005`, its scope shown as `owner`.
    It is the first check, before the command is looked up, and it applies to a dry run too.
    `RRS-C1005` with the scope `owner` is already how `token.login_code.create` and the owner-only settings keys refuse everyone but the owner, so no new catalog code is needed.
  - **What it means for `created_by`.** It is always the authenticated caller unless that caller is the owner: a token's `token.create` is recorded as `token:<token_id>`.
    The owner narrowed by a plugin UI is still recorded as that plugin, so disabling the plugin sweeps what its UI made.
  - **The migration's backfill takes a `plugin:` label at face value.** Before this fix, a row made by a token narrowed to a plugin reads the same as one made by the owner narrowed by that plugin's UI, and the journal keeps nothing that tells them apart.
    Such a token chains to the plugin, not to the narrowing token, so it is revoked when the plugin is disabled or uninstalled, or when it is named itself.
    Threat model R28 names this residual; `token.list` shows the token's creator as `plugin:<plugin_id>`, so the owner can see it.
  - flows/platform.md 3.2 (its text, the decision chart and the `run_command` sketch), 4.2's "Plugin UIs get no principal here" and 4.3's "Revoking follows what a token made" say the same.
    R28's "why it is open" names the code PR's test: a `system.admin` token that sets `acting_plugin_id` on `token.create` is refused with `RRS-C1005` and makes no token.
  - No new codes, commands, events, tables or settings.
  - **Choices for the owner's review:**
    - The refusal reuses `RRS-C1005` (scope `owner`) rather than a code of its own, as every other owner-only refusal does.
    - The backfill takes pre-fix `plugin:` labels at face value. The other simple rule would revoke every live token whose create row has a `plugin:` label, since no UI on `main` sends the field yet; it is safer but could revoke a token the owner made by hand.
- **SEC-01 (code):** builds 4.2's owner-only `acting_plugin_id` and 8.2's "Token creators and revocation" (threat model R28, now closed with its tests).
  - `tokens.created_by` loses its (M1) marker: migration `0003_token_creator.sql` adds it, fills it from the journal and revokes the successors already orphaned.
    Its test on a version-2 file is at the bottom of `crates/rastai-store/src/store.rs`, not in `tests/index_migrations.rs`: a file at an older version is made with the crate-private migration functions, as the newer-schema test already is. `tests/index_migrations.rs` checks the column against 8.2 and the version, 3.
  - New names in 15.1: `remove_scoped_token` (`http/sessions.rs`), the `Server` field `plugin_host_lock` (the plugin-host lock 8.2 names, a tokio mutex), `revoke_tokens_of_gone_plugins` (`plugin_host/lifecycle.rs`, the start sweep), `IndexWrite::RevokePluginTokens` and `StoreError::TokenCreatorRevoked`.
    `IndexWrite::RevokeToken` now sweeps the token's chain, and `IndexWrite::InsertToken` checks a `token:` creator in the writer's transaction.
  - `StoreError::TokenCreatorRevoked` is the one store error whose code isn't in the `RRS-S` range: it is the gate's `RRS-C1006`, which the refused caller gets.
  - flows/platform.md 2.1 shows the start sweep; layout rust-control-plane.md 6.2 (`IndexWrite`), 6.7, 8.2 (`Server`, `start_daemon`), 8.6 and 12.4 (`disable_plugin`, `revoke_tokens_of_gone_plugins`) say the same.
  - No new codes, commands, events, tables or settings.
  - **Choices for the owner's review:**
    - A `token:` creator with no row counts as dead, so its create is refused with `RRS-C1006` too: the gate admits only tokens `rastai.db` holds, so only a principal no request could carry has one.
    - The start sweep treats a plugin the host holds as `uninstalled` like one it doesn't hold, and a `disabled` one as gone whoever disabled it, as 8.2 says. At run time only `disable_plugin`'s own plugin is swept; a plugin its re-check turns off (`RRS-P1002`, `RRS-P3001`) keeps its tokens until the next start, and its principal holds no scope meanwhile.
    - When a disable's write fails it is logged and the plugin stays disabled in memory (its principal holds no scope); the next start sweeps its tokens while it is still disabled or gone. `disable_plugin` keeps its signature, since `plugin.disable` (M8) decides how the command reports it.
    - The gate's cache is brought up to date after a revoke by reading `tokens` back and marking every revoked row, rather than having the write report the ids it swept: the table is small and marking twice changes nothing.
- **SEC-01 (code, review round 1):** `IndexWrite::InsertToken` checks a `token:` creator only for a live row: a row inserted already revoked admits no one, and a rebuild copies revoked children of revoked or gone creators (8.2's "No gap for a racing create"). Tested by `crates/rastai-store/tests/token_creators.rs` `a_revoked_token_is_written_whatever_its_creator`.
