//! The `token` area's types (contracts 6.3, 6.4): the one-time login code, and scoped API tokens.
//!
//! A login code and a new token's text are credentials shown once, so their result fields are `secret`: the command
//! journal writes them as `"<redacted>"` in `result_json` (contracts 5.1), and only the caller ever sees them. An
//! idempotent replay of `token.create` returns that redacted result, so it never makes a second token.

use serde::{Deserialize, Serialize};

use crate::commands::{command_type, positional, secret};
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};

const LOGIN_CODE_CREATE_COMMAND_ID: &str = "token.login_code.create";
const CREATE_COMMAND_ID: &str = "token.create";
const LIST_COMMAND_ID: &str = "token.list";
const REVOKE_COMMAND_ID: &str = "token.revoke";
const TOKEN_SUMMARY_TYPE: &str = "TokenSummary";
/// The scoped-token presets of contracts 6.3; rastai-core's `token_preset_scopes` turns each into its scopes.
const TOKEN_PRESETS: &[&str] = &["chat-only", "mcp-default"];

/// `token.login_code.create` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenLoginCodeCreateArgs {}

/// A single-use code for the UI start URL's `#login=<code>` fragment (contracts 6.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenLoginCodeCreateResult {
    pub code: String,
    pub expires_unix_ms: u64,
    /// The daemon's HTTP port, so `rastai open` and `rastai tunnel` can build the URL.
    pub http_port: u32,
}

/// A new scoped token: a name, and its scopes as a preset or a list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCreateArgs {
    pub name: String,
    /// `chat-only` or `mcp-default`. Left out when empty, because the empty text isn't a preset.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preset: String,
    /// Scope names (contracts 6.3), added to the preset's.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// The new token. `token` is its text, shown this once; the daemon keeps only its hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCreateResult {
    pub token_id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub token: String,
}

/// One token as `token.list` shows it, never with its text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSummary {
    pub token_id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_unix_ms: u64,
    /// Who made it, as the journal names a principal: `owner`, `token:<token_id>` or `plugin:<plugin_id>`
    /// (contracts 6.3). Revoking that token or disabling that plugin revokes this one too (contracts 8.2, R28).
    /// Empty when nothing recorded the creator: a token made before migration 0003 with no journal row naming it.
    pub created_by: String,
    /// 0 while the token is active.
    #[serde(default)]
    pub revoked_unix_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenListArgs {
    /// Also list revoked tokens.
    #[serde(default)]
    pub include_revoked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenListResult {
    pub tokens: Vec<TokenSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRevokeArgs {
    pub token_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRevokeResult {
    pub token_id: String,
    pub revoked_unix_ms: u64,
}

pub fn describe_token_login_code_create_args() -> TypeDescription {
    command_type(
        "TokenLoginCodeCreateArgs",
        LOGIN_CODE_CREATE_COMMAND_ID,
        "token.login_code.create takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_token_login_code_create_result() -> TypeDescription {
    let fields = vec![
        secret(field(
            "code",
            FieldKind::String,
            REQUIRED,
            "The single-use login code, for the #login=<code> fragment.",
        )),
        field(
            "expires_unix_ms",
            FieldKind::Integer,
            REQUIRED,
            "When the code stops working, 60 seconds after it was made.",
        ),
        field("http_port", FieldKind::Integer, REQUIRED, "The daemon's HTTP port."),
    ];
    command_type(
        "TokenLoginCodeCreateResult",
        LOGIN_CODE_CREATE_COMMAND_ID,
        "A single-use login code for the UI start URL.",
        fields,
    )
}

pub fn describe_token_create_args() -> TypeDescription {
    let fields = vec![
        positional(
            0,
            field(
                "name",
                FieldKind::String,
                REQUIRED,
                "A name to recognise the token by, such as claude-desktop.",
            ),
        ),
        field(
            "preset",
            enum_of(TOKEN_PRESETS),
            OPTIONAL,
            "A scope preset: chat-only or mcp-default.",
        ),
        field(
            "scopes",
            list_of(FieldKind::String),
            OPTIONAL,
            "Scope names, added to the preset's.",
        ),
    ];
    command_type(
        "TokenCreateArgs",
        CREATE_COMMAND_ID,
        "A new scoped API token: a name, and its scopes as a preset or a list.",
        fields,
    )
}

pub fn describe_token_create_result() -> TypeDescription {
    let fields = vec![
        field("token_id", FieldKind::String, REQUIRED, "The token's id."),
        field("name", FieldKind::String, REQUIRED, "The token's name."),
        field("scopes", list_of(FieldKind::String), REQUIRED, "The scopes it holds."),
        secret(field(
            "token",
            FieldKind::String,
            REQUIRED,
            "The token's text, shown this once; the daemon keeps only its hash.",
        )),
    ];
    command_type(
        "TokenCreateResult",
        CREATE_COMMAND_ID,
        "The new token, with its text shown this once.",
        fields,
    )
}

pub fn describe_token_summary() -> TypeDescription {
    let fields = vec![
        field("token_id", FieldKind::String, REQUIRED, "The token's id."),
        field("name", FieldKind::String, REQUIRED, "The token's name."),
        field("scopes", list_of(FieldKind::String), REQUIRED, "The scopes it holds."),
        field("created_unix_ms", FieldKind::Integer, REQUIRED, "When it was made."),
        field(
            "created_by",
            FieldKind::String,
            REQUIRED,
            "Who made it: owner, token:<token_id> or plugin:<plugin_id>; empty when no record names the creator.",
        ),
        field(
            "revoked_unix_ms",
            FieldKind::Integer,
            OPTIONAL,
            "When it was revoked; 0 or absent while it is active.",
        ),
    ];
    command_type(
        TOKEN_SUMMARY_TYPE,
        LIST_COMMAND_ID,
        "One API token, never with its text.",
        fields,
    )
}

pub fn describe_token_list_args() -> TypeDescription {
    let fields = vec![field(
        "include_revoked",
        FieldKind::Boolean,
        OPTIONAL,
        "Also list revoked tokens.",
    )];
    command_type("TokenListArgs", LIST_COMMAND_ID, "Which tokens to list.", fields)
}

pub fn describe_token_list_result() -> TypeDescription {
    let fields = vec![field(
        "tokens",
        list_of(FieldKind::Type(TOKEN_SUMMARY_TYPE.to_string())),
        REQUIRED,
        "The tokens, oldest first.",
    )];
    command_type("TokenListResult", LIST_COMMAND_ID, "The API tokens.", fields)
}

pub fn describe_token_revoke_args() -> TypeDescription {
    let fields = vec![positional(
        0,
        field("token_id", FieldKind::String, REQUIRED, "The token to revoke."),
    )];
    command_type("TokenRevokeArgs", REVOKE_COMMAND_ID, "Which token to revoke.", fields)
}

pub fn describe_token_revoke_result() -> TypeDescription {
    let fields = vec![
        field("token_id", FieldKind::String, REQUIRED, "The revoked token."),
        field("revoked_unix_ms", FieldKind::Integer, REQUIRED, "When it was revoked."),
    ];
    command_type(
        "TokenRevokeResult",
        REVOKE_COMMAND_ID,
        "The revoked token. It stops working at once.",
        fields,
    )
}
