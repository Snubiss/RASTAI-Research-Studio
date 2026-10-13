//! The `token` commands (contracts 6.3, 6.4): the one-time login code, and scoped API tokens for scripts and MCP
//! clients (flows/platform.md 4.3).
//!
//! A token is 32 random bytes written as 64 lowercase hex characters, the same form as the owner token. Only its
//! BLAKE3 hash is stored (`tokens.token_hash`) and kept in the gate's cache; its text is in `token.create`'s answer
//! and nowhere else, since the journal redacts that field (`secret`, contracts 5.1). A revoked token is refused on
//! its next request (`RRS-C1006`). Closing a revoked token's open streams joins with the streams (M1-28).
//!
//! Only the owner creates tokens with any scope (flows/platform.md 4.3). Any other principal that reaches
//! `token.create` (a token or a plugin holding `system.admin`) can only hand out scopes it holds itself, so no token
//! is ever a way to more rights than its creator had (`RRS-C1005`, the same code as a missing scope).
//!
//! Every token records its creator (`tokens.created_by`, the journal's principal text), and revocation follows it
//! (contracts 8.2 "Token creators and revocation", threat model R28): revoking a token revokes, in the same write,
//! every live token it made and every token those made, and disabling a plugin does the same for what the plugin
//! made (plugin_host/lifecycle.rs). A create never outlives its creator's revoke:
//! - a `token:` creator is checked in the writer transaction that inserts the row, the one every revoke goes
//!   through, so the create is refused with `RRS-C1006` once the creator is revoked;
//! - a `plugin:` creator is checked against the plugin host under `Server.plugin_host_lock`, held until the insert
//!   commits; `disable_plugin` holds the same lock until its sweep is in the gate's cache (`RRS-C1005` once the
//!   plugin isn't enabled, as its grant is then empty).
//!
//! The gate's cache follows the database: a new token goes into the cache before its insert is sent (nobody holds
//! its text until `token.create` answers) and comes out again if the insert fails, and a revoke marks its tokens only
//! after its write committed. So a sweep always finds the entry it must mark, and no create puts a live one back.
//! The token `token.revoke` names is marked without a read; the rest of a sweep is read back from `rastai.db`, and
//! when that read fails every scoped token is marked revoked, so the gate fails closed until the next start.
//!
//! A login code is owner-only (flows/platform.md 4.1): its session is the owner's, with every scope, so a code made
//! for a token or a plugin, even one holding `system.admin`, would turn that principal into the owner. Every principal
//! but the owner is refused `token.login_code.create` with `RRS-C1005`, dry runs included.

use rastai_core::commands::output::CommandOutput;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::principal::{Principal, PrincipalKind, principal_label};
use rastai_core::random::random_bytes;
use rastai_core::scopes::{Scope, parse_scope, scope_name, scopes_allow, token_preset_scopes};
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::{Diagnostic, diagnostic, diagnostic_at};
use rastai_diag::logger::log_warn;
use rastai_formats::commands::result_json;
use rastai_formats::commands::token::{
    TokenCreateArgs, TokenCreateResult, TokenListArgs, TokenListResult, TokenLoginCodeCreateArgs,
    TokenLoginCodeCreateResult, TokenRevokeArgs, TokenRevokeResult, TokenSummary,
};
use rastai_formats::hashing::hash_bytes;
use rastai_proto::common::v1::PluginState;
use rastai_store::error::StoreError;
use rastai_store::index::kept_tables::{TokenRow, list_tokens};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use tokio::sync::MutexGuard;

use crate::auth_files::lowercase_hex;
use crate::commands::arguments::invalid_arguments;
use crate::commands::context::{CommandContext, core_failure, store_failure, sync_result};
use crate::http::sessions::{
    AuthState, ScopedToken, add_scoped_token, create_login_code, remove_scoped_token, revoke_all_scoped_tokens,
    revoke_scoped_token,
};
use crate::plugin_host::load::plugin_status;
use crate::server_state::Server;

/// The same strength as the owner token, login codes and session keys.
const TOKEN_SECRET_BYTES: usize = 32;
const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
const CREATE_COMMAND_ID: &str = "token.create";
const LOGIN_CODE_COMMAND_ID: &str = "token.login_code.create";
/// What `RRS-C1005` names as the needed scope of an owner-only command: no scope grants it.
const OWNER_ONLY: &str = "owner";
const NAME_PATH: &str = "#/name";
const SCOPES_PATH: &str = "#/scopes";
const TOKEN_ID_PATH: &str = "#/token_id";
const EXPECTED_NAME: &str = "a name to recognise the token by, such as claude-desktop";
const EXPECTED_SOME_SCOPE: &str = "a preset or at least one scope";
const EXPECTED_TOKEN_ID: &str = "the id of a token token.list shows";
/// How a refusal shows text that is empty.
const EMPTY_TEXT: &str = "(blank)";

/// A single-use code for the UI start URL, valid for 60 seconds, and the port to open it on. A dry run makes none:
/// its answer has the port and an empty code.
pub(crate) async fn login_code_create(
    context: &CommandContext<'_>,
    _arguments: TokenLoginCodeCreateArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    check_owner(context.principal, LOGIN_CODE_COMMAND_ID)?;
    if context.dry_run {
        let planned = TokenLoginCodeCreateResult {
            code: String::new(),
            expires_unix_ms: 0,
            http_port: u32::from(http_port(context.server)),
        };
        return Ok(sync_result(result_json(&planned)));
    }
    let new_code = create_login_code(&context.server.auth, now_unix_ms()).map_err(|error| core_failure(&error))?;
    let result = TokenLoginCodeCreateResult {
        code: new_code.code,
        expires_unix_ms: u64::try_from(new_code.expires_unix_ms).unwrap_or(0),
        http_port: u32::from(http_port(context.server)),
    };
    Ok(sync_result(result_json(&result)))
}

/// A new scoped token: the preset's scopes plus the listed ones, each one the caller may grant. A dry run checks
/// them and makes no token: its answer lists the scopes, with an empty id and token.
pub(crate) async fn create(
    context: &CommandContext<'_>,
    arguments: TokenCreateArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    check_name(&arguments.name)?;
    let scopes = requested_scopes(&arguments)?;
    check_grantable(context.principal, &scopes)?;
    let scope_names: Vec<String> = scopes.iter().map(scope_name).collect();
    if context.dry_run {
        let planned = TokenCreateResult {
            token_id: String::new(),
            name: arguments.name,
            scopes: scope_names,
            token: String::new(),
        };
        return Ok(sync_result(result_json(&planned)));
    }
    // Held until the insert has committed, so a disable can't sweep between the check and the insert (contracts 8.2).
    let _plugin_host_guard = lock_plugin_creator(context.server, context.principal, &context.command.scope).await?;
    pause_if_a_test_asked(context.server).await;
    let token_id = new_id(IdPrefix::Token).map_err(|error| core_failure(&error))?;
    let secret = random_bytes(TOKEN_SECRET_BYTES).map_err(|error| core_failure(&error))?;
    let token = lowercase_hex(&secret);
    let row = TokenRow {
        token_id: token_id.clone(),
        name: arguments.name.clone(),
        token_hash: hash_bytes(token.as_bytes()),
        scopes: scope_names.clone(),
        created_unix_ms: now_unix_ms(),
        revoked_unix_ms: None,
        created_by: principal_label(context.principal),
    };
    insert_token(context.server, row, scopes).await?;
    let result = TokenCreateResult {
        token_id,
        name: arguments.name,
        scopes: scope_names,
        token,
    };
    Ok(sync_result(result_json(&result)))
}

/// For a `plugin:` creator, takes the plugin-host lock and checks the plugin is still enabled; the caller holds the
/// guard until its insert commits. Any other creator needs no lock: a `token:` one is checked by the insert itself.
async fn lock_plugin_creator<'server>(
    server: &'server Server,
    creator: &Principal,
    scope: &str,
) -> Result<Option<MutexGuard<'server, ()>>, Vec<Diagnostic>> {
    if creator.kind != PrincipalKind::Plugin {
        return Ok(None);
    }
    let guard = server.plugin_host_lock.lock().await;
    check_creator_plugin_enabled(server, creator, scope)?;
    Ok(Some(guard))
}

/// The test seam `Server.token_create_pause`: a test stops a create here, between its checks and its insert, to put
/// a disable in that window. Nothing happens unless a test set it.
async fn pause_if_a_test_asked(server: &Server) {
    let pause = match server.token_create_pause.lock() {
        Ok(mut pause) => pause.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    let Some(pause) = pause else {
        return;
    };
    if pause.reached.send(()).is_ok() {
        // Resumed or dropped by the test, the create goes on either way.
        let _ = pause.resume.await;
    }
}

/// Adds the token to the gate's cache, then inserts its row; takes the cache entry back if the insert fails or is
/// refused (`RRS-C1006` when its `token:` creator was revoked meanwhile).
async fn insert_token(server: &Server, row: TokenRow, scopes: Vec<Scope>) -> Result<(), Vec<Diagnostic>> {
    let token_id = row.token_id.clone();
    let cached = ScopedToken {
        token_id: token_id.clone(),
        token_hash: row.token_hash.clone(),
        scopes,
        revoked: false,
    };
    add_scoped_token(&server.auth, cached);
    if let Err(error) = write_index(&server.store.index, IndexWrite::InsertToken(row)).await {
        remove_scoped_token(&server.auth, &token_id);
        return Err(store_failure(&error));
    }
    Ok(())
}

/// The tokens, oldest first, never with their text; revoked ones only when asked for.
pub(crate) async fn list(
    context: &CommandContext<'_>,
    arguments: TokenListArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let rows = read_tokens(context.server)?;
    let tokens = rows
        .iter()
        .filter(|row| arguments.include_revoked || row.revoked_unix_ms.is_none())
        .map(token_summary)
        .collect();
    Ok(sync_result(result_json(&TokenListResult { tokens })))
}

/// Revokes a token at once, with every live token it made, directly or through tokens it made (contracts 8.2).
/// Revoking one already revoked keeps its first time and still sweeps what is live under it. A dry run checks the
/// id only.
pub(crate) async fn revoke(
    context: &CommandContext<'_>,
    arguments: TokenRevokeArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let rows = read_tokens(context.server)?;
    let Some(row) = rows.iter().find(|row| row.token_id == arguments.token_id) else {
        let found = shown_text(&arguments.token_id);
        let refusal = diagnostic_at(
            CODE_INVALID_ARGUMENTS,
            &[("found", &found), ("expected", EXPECTED_TOKEN_ID)],
            TOKEN_ID_PATH,
        );
        return Err(vec![refusal]);
    };
    let revoked_unix_ms = if context.dry_run {
        row.revoked_unix_ms.unwrap_or_else(now_unix_ms)
    } else {
        revoke_now(context.server, row).await?
    };
    let result = TokenRevokeResult {
        token_id: row.token_id.clone(),
        revoked_unix_ms: u64::try_from(revoked_unix_ms).unwrap_or(0),
    };
    Ok(sync_result(result_json(&result)))
}

/// Revokes the token's whole chain in one write, then marks it in the gate's cache. Returns the named token's revoke
/// time: its first one when it was already revoked. Once the write has committed the revoke has happened, so the
/// answer is a success even when the cache had to fail closed.
async fn revoke_now(server: &Server, row: &TokenRow) -> Result<i64, Vec<Diagnostic>> {
    let revoked_unix_ms = now_unix_ms();
    let write = IndexWrite::RevokeToken {
        token_id: row.token_id.clone(),
        revoked_unix_ms,
    };
    write_index(&server.store.index, write)
        .await
        .map_err(|error| store_failure(&error))?;
    mark_revoked_tokens_in_cache(server, Some(&row.token_id));
    Ok(row.revoked_unix_ms.unwrap_or(revoked_unix_ms))
}

/// Puts a revoke whose write has committed into the gate's cache, so every token it swept is refused (`RRS-C1006`)
/// from its next request on. `named_token_id` is the token `token.revoke` named; a plugin's sweep names none. The
/// rest of the sweep is read back from `rastai.db`: marking a token twice changes nothing, so this reads the small
/// table whole rather than asking the write which ids it swept.
pub(crate) fn mark_revoked_tokens_in_cache(server: &Server, named_token_id: Option<&str>) {
    let rows_read_back = read_token_rows(server);
    if let Err(error) = &rows_read_back {
        log_warn(
            "the revoked tokens could not be read back, so every scoped token is refused until the daemon restarts",
            &[("code", error.code())],
        );
    }
    apply_committed_revoke(&server.auth, named_token_id, rows_read_back);
}

/// The order that keeps a committed revoke from leaving a usable token: the named token first, as it needs no read;
/// then every row read back as revoked. When the read failed, the cache can't tell which tokens the sweep reached, so
/// every scoped token is marked revoked: the gate fails closed until the next start loads the cache from `rastai.db`.
fn apply_committed_revoke(
    auth: &AuthState,
    named_token_id: Option<&str>,
    rows_read_back: Result<Vec<TokenRow>, StoreError>,
) {
    if let Some(token_id) = named_token_id {
        revoke_scoped_token(auth, token_id);
    }
    let Ok(rows) = rows_read_back else {
        revoke_all_scoped_tokens(auth);
        return;
    };
    for row in rows {
        if row.revoked_unix_ms.is_some() {
            revoke_scoped_token(auth, &row.token_id);
        }
    }
}

fn read_token_rows(server: &Server) -> Result<Vec<TokenRow>, StoreError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    list_tokens(&reader)
}

/// A plugin, or the owner narrowed to its UI, makes a token only while the plugin is enabled. The narrowing already
/// empties a plugin's grant when it isn't, but the plugin may be disabled between that check and this one; the
/// caller holds the plugin-host lock from here until its insert commits.
fn check_creator_plugin_enabled(server: &Server, creator: &Principal, scope: &str) -> Result<(), Vec<Diagnostic>> {
    let enabled = matches!(plugin_status(server, &creator.id), Some((PluginState::Enabled, _)));
    if enabled {
        return Ok(());
    }
    let label = principal_label(creator);
    let refusal = diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[
            ("command_id", CREATE_COMMAND_ID),
            ("scope", scope),
            ("principal", &label),
        ],
    );
    Err(vec![refusal])
}

fn check_name(name: &str) -> Result<(), Vec<Diagnostic>> {
    if !name.trim().is_empty() {
        return Ok(());
    }
    let refusal = diagnostic_at(
        CODE_INVALID_ARGUMENTS,
        &[("found", EMPTY_TEXT), ("expected", EXPECTED_NAME)],
        NAME_PATH,
    );
    Err(vec![refusal])
}

/// The preset's scopes, then each listed scope once. A scope name that isn't one is refused at its place in the
/// list, after `RRS-C0002` like any argument problem; a token with no scope at all would be useless, so it is
/// refused too.
fn requested_scopes(arguments: &TokenCreateArgs) -> Result<Vec<Scope>, Vec<Diagnostic>> {
    let mut scopes = if arguments.preset.is_empty() {
        Vec::new()
    } else {
        // Layer 2 already refused any word but the two presets.
        token_preset_scopes(&arguments.preset).unwrap_or_default()
    };
    let mut refusals = Vec::new();
    for (index, name) in arguments.scopes.iter().enumerate() {
        match parse_scope(name) {
            Ok(scope) if scopes.contains(&scope) => {}
            Ok(scope) => scopes.push(scope),
            Err(mut refusal) => {
                refusal.path = format!("{SCOPES_PATH}/{index}");
                refusals.push(refusal);
            }
        }
    }
    if !refusals.is_empty() {
        return Err(invalid_arguments(CREATE_COMMAND_ID, refusals));
    }
    if scopes.is_empty() {
        let refusal = diagnostic_at(
            CODE_INVALID_ARGUMENTS,
            &[("found", "no scope"), ("expected", EXPECTED_SOME_SCOPE)],
            SCOPES_PATH,
        );
        return Err(vec![refusal]);
    }
    Ok(scopes)
}

/// The owner may grant every scope. Anyone else may grant only the scopes it holds: `system.admin` lets it run
/// `token.create`, but doesn't grant `chat` (scopes.rs), so it can't make a token that has `chat`. Every scope it
/// lacks is refused, so the caller sees them all at once.
fn check_grantable(creator: &Principal, scopes: &[Scope]) -> Result<(), Vec<Diagnostic>> {
    if creator.kind == PrincipalKind::Owner {
        return Ok(());
    }
    let label = principal_label(creator);
    let refusals: Vec<Diagnostic> = scopes
        .iter()
        .filter(|scope| !scopes_allow(&creator.scopes, scope))
        .map(|scope| scope_not_granted(scope, &label))
        .collect();
    if refusals.is_empty() { Ok(()) } else { Err(refusals) }
}

/// A login code's session runs as the owner (http/gate.rs), so only the owner may ask for one. A token or a plugin
/// holding `system.admin` reaches the handler past the scope check and is refused here.
fn check_owner(caller: &Principal, command_id: &str) -> Result<(), Vec<Diagnostic>> {
    if caller.kind == PrincipalKind::Owner {
        return Ok(());
    }
    let label = principal_label(caller);
    let refusal = diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[("command_id", command_id), ("scope", OWNER_ONLY), ("principal", &label)],
    );
    Err(vec![refusal])
}

fn scope_not_granted(scope: &Scope, creator_label: &str) -> Diagnostic {
    let name = scope_name(scope);
    diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[
            ("command_id", CREATE_COMMAND_ID),
            ("scope", &name),
            ("principal", creator_label),
        ],
    )
}

fn read_tokens(server: &Server) -> Result<Vec<TokenRow>, Vec<Diagnostic>> {
    // A small table on a pooled read-only connection; it never waits on the writer (WAL).
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    list_tokens(&reader).map_err(|error| store_failure(&error))
}

fn token_summary(row: &TokenRow) -> TokenSummary {
    TokenSummary {
        token_id: row.token_id.clone(),
        name: row.name.clone(),
        scopes: row.scopes.clone(),
        created_unix_ms: u64::try_from(row.created_unix_ms).unwrap_or(0),
        created_by: row.created_by.clone(),
        revoked_unix_ms: row
            .revoked_unix_ms
            .map(|unix_ms| u64::try_from(unix_ms).unwrap_or(0))
            .unwrap_or(0),
    }
}

/// The loopback port, once the listener is bound; 0 before, which only a command run during startup could see.
fn http_port(server: &Server) -> u16 {
    server.http_port.get().copied().unwrap_or(0)
}

fn shown_text(text: &str) -> String {
    if text.is_empty() {
        EMPTY_TEXT.to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn create_arguments(preset: &str, scopes: &[&str]) -> TokenCreateArgs {
        TokenCreateArgs {
            name: "ci".to_string(),
            preset: preset.to_string(),
            scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
        }
    }

    #[test]
    fn scopes_are_the_preset_plus_the_list_each_once() {
        let scopes = requested_scopes(&create_arguments("chat-only", &["read", "chat", "read"])).unwrap();
        assert_eq!(scopes, vec![Scope::Chat, Scope::Read]);
        let plugin = requested_scopes(&create_arguments("", &["plugin:acme"])).unwrap();
        assert_eq!(plugin, vec![Scope::Plugin("acme".to_string())]);
    }

    #[test]
    fn unknown_scopes_are_refused_at_their_place() {
        let refusal = requested_scopes(&create_arguments("", &["read", "everything", "root"])).unwrap_err();
        let found: Vec<(&str, &str)> = refusal
            .iter()
            .map(|item| (item.code.as_str(), item.path.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("RRS-C0002", "#/scopes/1"),
                ("RRS-E1004", "#/scopes/1"),
                ("RRS-E1004", "#/scopes/2")
            ]
        );
    }

    #[test]
    fn a_token_needs_some_scope_and_a_name() {
        let refusal = requested_scopes(&create_arguments("", &[])).unwrap_err();
        assert_eq!(
            (refusal[0].code.as_str(), refusal[0].path.as_str()),
            ("RRS-C0002", SCOPES_PATH)
        );
        let refusal = check_name("  ").unwrap_err();
        assert_eq!(
            (refusal[0].code.as_str(), refusal[0].path.as_str()),
            ("RRS-C0002", NAME_PATH)
        );
        check_name("claude-desktop").unwrap();
    }

    fn token_holding(scopes: Vec<Scope>) -> Principal {
        Principal {
            kind: PrincipalKind::Token,
            id: "tok_admin".to_string(),
            scopes,
        }
    }

    #[test]
    fn only_the_owner_grants_scopes_it_was_not_given() {
        let wanted = vec![Scope::Read, Scope::Chat, Scope::Secrets];
        check_grantable(&rastai_core::principal::owner_principal(), &wanted).unwrap();
        let admin = token_holding(vec![Scope::SystemAdmin, Scope::Read]);
        let refusals = check_grantable(&admin, &wanted).unwrap_err();
        let codes: Vec<&str> = refusals.iter().map(|item| item.code.as_str()).collect();
        assert_eq!(codes, ["RRS-C1005", "RRS-C1005"]);
        assert!(refusals[0].message.contains("chat"), "{}", refusals[0].message);
        assert!(refusals[0].message.contains("tok_admin"), "{}", refusals[0].message);
        assert!(refusals[1].message.contains("secrets"), "{}", refusals[1].message);
        // What it holds, it may pass on, its own system.admin included.
        check_grantable(&admin, &[Scope::Read, Scope::SystemAdmin]).unwrap();
    }

    #[test]
    fn only_the_owner_may_ask_for_a_login_code() {
        check_owner(&rastai_core::principal::owner_principal(), LOGIN_CODE_COMMAND_ID).unwrap();
        let every_scope = vec![Scope::SystemAdmin, Scope::Read, Scope::Chat, Scope::Secrets];
        let plugin = Principal {
            kind: PrincipalKind::Plugin,
            id: "acme".to_string(),
            scopes: every_scope.clone(),
        };
        for caller in [token_holding(every_scope), plugin] {
            let refusal = check_owner(&caller, LOGIN_CODE_COMMAND_ID).unwrap_err();
            assert_eq!(refusal[0].code, CODE_SCOPE_NOT_GRANTED);
            assert!(
                refusal[0].message.contains(LOGIN_CODE_COMMAND_ID),
                "{}",
                refusal[0].message
            );
            assert!(
                refusal[0].message.contains(&principal_label(&caller)),
                "{}",
                refusal[0].message
            );
        }
    }

    fn cached_token(token_id: &str) -> ScopedToken {
        ScopedToken {
            token_id: token_id.to_string(),
            token_hash: format!("blake3:{token_id}"),
            scopes: vec![Scope::Read],
            revoked: false,
        }
    }

    fn stored_token(token_id: &str, revoked_unix_ms: Option<i64>) -> TokenRow {
        TokenRow {
            token_id: token_id.to_string(),
            name: token_id.to_string(),
            token_hash: format!("blake3:{token_id}"),
            scopes: vec!["read".to_string()],
            created_unix_ms: 1,
            revoked_unix_ms,
            created_by: "owner".to_string(),
        }
    }

    /// The gate's cache with tok_named, its child tok_child and the unrelated tok_other, all live.
    fn auth_with_three_tokens() -> AuthState {
        let auth = crate::http::sessions::new_auth_state("blake3:owner".to_string(), "0123abcd");
        for token_id in ["tok_named", "tok_child", "tok_other"] {
            add_scoped_token(&auth, cached_token(token_id));
        }
        auth
    }

    fn revoked_in_cache(auth: &AuthState, token_id: &str) -> bool {
        let hash = format!("blake3:{token_id}");
        crate::http::sessions::find_scoped_token(auth, &hash).unwrap().revoked
    }

    #[test]
    fn a_committed_revoke_marks_the_named_token_and_what_the_read_shows_revoked() {
        let auth = auth_with_three_tokens();
        // The write swept tok_child with tok_named. tok_named's row is left live in the read back, so only the
        // step that needs no read can mark it.
        let rows = vec![
            stored_token("tok_named", None),
            stored_token("tok_child", Some(5)),
            stored_token("tok_other", None),
        ];
        apply_committed_revoke(&auth, Some("tok_named"), Ok(rows));
        assert!(revoked_in_cache(&auth, "tok_named"));
        assert!(revoked_in_cache(&auth, "tok_child"));
        assert!(!revoked_in_cache(&auth, "tok_other"));
    }

    #[test]
    fn a_committed_revoke_whose_read_back_fails_refuses_every_scoped_token() {
        let auth = auth_with_three_tokens();
        let read_failed = StoreError::DatabaseReadFailed {
            reason: "disk I/O error".to_string(),
        };
        apply_committed_revoke(&auth, Some("tok_named"), Err(read_failed));
        for token_id in ["tok_named", "tok_child", "tok_other"] {
            assert!(revoked_in_cache(&auth, token_id), "{token_id}");
        }
    }

    #[test]
    fn a_summary_shows_its_creator_and_an_active_token_as_revoked_at_zero() {
        let row = TokenRow {
            token_id: "tok_a".to_string(),
            name: "ci".to_string(),
            token_hash: "blake3:00".to_string(),
            scopes: vec!["read".to_string()],
            created_unix_ms: 5,
            revoked_unix_ms: None,
            created_by: "token:tok_parent".to_string(),
        };
        let summary = token_summary(&row);
        assert_eq!(summary.created_unix_ms, 5);
        assert_eq!(summary.created_by, "token:tok_parent");
        assert_eq!(summary.revoked_unix_ms, 0);
        let revoked = TokenRow {
            revoked_unix_ms: Some(9),
            ..row
        };
        assert_eq!(token_summary(&revoked).revoked_unix_ms, 9);
    }
}
