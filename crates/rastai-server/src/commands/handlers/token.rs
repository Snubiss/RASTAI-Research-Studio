//! The `token` commands (contracts 6.3, 6.4): the one-time login code, and scoped API tokens for scripts and MCP
//! clients (flows/platform.md 4.3).
//!
//! A token is 32 random bytes written as 64 lowercase hex characters, the same form as the owner token. Only its
//! BLAKE3 hash is stored (`tokens.token_hash`) and kept in the gate's cache; its text is in `token.create`'s answer
//! and nowhere else, since the journal redacts that field (`secret`, contracts 5.1). A revoked token is refused on
//! its next request (`RRS-C1006`). Closing a revoked token's open streams joins with the streams (M1-28).

use rastai_core::commands::output::CommandOutput;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::random::random_bytes;
use rastai_core::scopes::{Scope, parse_scope, scope_name, token_preset_scopes};
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_formats::commands::result_json;
use rastai_formats::commands::token::{
    TokenCreateArgs, TokenCreateResult, TokenListArgs, TokenListResult, TokenLoginCodeCreateArgs,
    TokenLoginCodeCreateResult, TokenRevokeArgs, TokenRevokeResult, TokenSummary,
};
use rastai_formats::hashing::hash_bytes;
use rastai_store::index::kept_tables::{TokenRow, list_tokens};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;

use crate::auth_files::lowercase_hex;
use crate::commands::arguments::invalid_arguments;
use crate::commands::context::{CommandContext, core_failure, store_failure, sync_result};
use crate::http::sessions::{ScopedToken, add_scoped_token, create_login_code, revoke_scoped_token};
use crate::server_state::Server;

/// The same strength as the owner token, login codes and session keys.
const TOKEN_SECRET_BYTES: usize = 32;
const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CREATE_COMMAND_ID: &str = "token.create";
const NAME_PATH: &str = "#/name";
const SCOPES_PATH: &str = "#/scopes";
const TOKEN_ID_PATH: &str = "#/token_id";
const EXPECTED_NAME: &str = "a name to recognise the token by, such as claude-desktop";
const EXPECTED_SOME_SCOPE: &str = "a preset or at least one scope";
const EXPECTED_TOKEN_ID: &str = "the id of a token token.list shows";
/// How a refusal shows text that is empty.
const EMPTY_TEXT: &str = "(blank)";

/// A single-use code for the UI start URL, valid for 60 seconds, and the port to open it on. A dry run makes none.
pub(crate) async fn login_code_create(
    context: &CommandContext<'_>,
    _arguments: TokenLoginCodeCreateArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    if context.dry_run {
        return Ok(CommandOutput::default());
    }
    let new_code = create_login_code(&context.server.auth, now_unix_ms()).map_err(|error| core_failure(&error))?;
    let result = TokenLoginCodeCreateResult {
        code: new_code.code,
        expires_unix_ms: u64::try_from(new_code.expires_unix_ms).unwrap_or(0),
        http_port: u32::from(http_port(context.server)),
    };
    Ok(sync_result(result_json(&result)))
}

/// A new scoped token: the preset's scopes plus the listed ones. A dry run checks them and makes no token.
pub(crate) async fn create(
    context: &CommandContext<'_>,
    arguments: TokenCreateArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    check_name(&arguments.name)?;
    let scopes = requested_scopes(&arguments)?;
    if context.dry_run {
        return Ok(CommandOutput::default());
    }
    let token_id = new_id(IdPrefix::Token).map_err(|error| core_failure(&error))?;
    let secret = random_bytes(TOKEN_SECRET_BYTES).map_err(|error| core_failure(&error))?;
    let token = lowercase_hex(&secret);
    let scope_names: Vec<String> = scopes.iter().map(scope_name).collect();
    let row = TokenRow {
        token_id: token_id.clone(),
        name: arguments.name.clone(),
        token_hash: hash_bytes(token.as_bytes()),
        scopes: scope_names.clone(),
        created_unix_ms: now_unix_ms(),
        revoked_unix_ms: None,
    };
    let cached = ScopedToken {
        token_id: token_id.clone(),
        token_hash: row.token_hash.clone(),
        scopes,
        revoked: false,
    };
    write_index(&context.server.store.index, IndexWrite::InsertToken(row))
        .await
        .map_err(|error| store_failure(&error))?;
    add_scoped_token(&context.server.auth, cached);
    let result = TokenCreateResult {
        token_id,
        name: arguments.name,
        scopes: scope_names,
        token,
    };
    Ok(sync_result(result_json(&result)))
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

/// Revokes a token at once. Revoking one already revoked keeps its first time. A dry run checks the id only.
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
    let revoked_unix_ms = match row.revoked_unix_ms {
        Some(earlier) => earlier,
        None if context.dry_run => now_unix_ms(),
        None => revoke_now(context.server, &row.token_id).await?,
    };
    let result = TokenRevokeResult {
        token_id: row.token_id.clone(),
        revoked_unix_ms: u64::try_from(revoked_unix_ms).unwrap_or(0),
    };
    Ok(sync_result(result_json(&result)))
}

async fn revoke_now(server: &Server, token_id: &str) -> Result<i64, Vec<Diagnostic>> {
    let revoked_unix_ms = now_unix_ms();
    let write = IndexWrite::RevokeToken {
        token_id: token_id.to_string(),
        revoked_unix_ms,
    };
    write_index(&server.store.index, write)
        .await
        .map_err(|error| store_failure(&error))?;
    revoke_scoped_token(&server.auth, token_id);
    Ok(revoked_unix_ms)
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

    #[test]
    fn a_summary_shows_an_active_token_as_revoked_at_zero() {
        let row = TokenRow {
            token_id: "tok_a".to_string(),
            name: "ci".to_string(),
            token_hash: "blake3:00".to_string(),
            scopes: vec!["read".to_string()],
            created_unix_ms: 5,
            revoked_unix_ms: None,
        };
        let summary = token_summary(&row);
        assert_eq!(summary.created_unix_ms, 5);
        assert_eq!(summary.revoked_unix_ms, 0);
        let revoked = TokenRow {
            revoked_unix_ms: Some(9),
            ..row
        };
        assert_eq!(token_summary(&revoked).revoked_unix_ms, 9);
    }
}
