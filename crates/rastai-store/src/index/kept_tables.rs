//! Rows of `projects`, `tokens`, `nodes` and `plugins` (contracts 8.2): the tables a rebuild copies from the old
//! file instead of rebuilding from the logs, because no log records them (layout section 13.2).
//!
//! The list columns (`tokens.scopes`, `nodes.capabilities`, `plugins.granted_permissions`) hold JSON arrays of text.

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{
    SqlValue, SqliteTransaction, execute, integer_value, optional_integer_value, query_rows, read_integer,
    read_optional_integer, read_text, text_value,
};

const UPSERT_PROJECT: &str = "INSERT INTO projects (project_id, path, name, opened_unix_ms)
    VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT (project_id) DO UPDATE SET path = excluded.path, name = excluded.name,
        opened_unix_ms = excluded.opened_unix_ms";
const SELECT_PROJECT_BY_ID: &str = "SELECT project_id, path, name, opened_unix_ms FROM projects WHERE project_id = ?1";

// A token is inserted once and never changed but for its revocation, so a duplicate id is an error, not an update.
const INSERT_TOKEN: &str = "INSERT INTO tokens (token_id, name, token_hash, scopes, created_unix_ms, revoked_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
// Revoking twice keeps the first time.
const REVOKE_TOKEN: &str = "UPDATE tokens SET revoked_unix_ms = ?2 WHERE token_id = ?1 AND revoked_unix_ms IS NULL";
const SELECT_TOKEN_BY_HASH: &str = "SELECT token_id, name, token_hash, scopes, created_unix_ms, revoked_unix_ms
    FROM tokens WHERE token_hash = ?1";
// Oldest first, as token.list shows them; the id breaks a tie, since ids sort by creation time too.
const SELECT_ALL_TOKENS: &str = "SELECT token_id, name, token_hash, scopes, created_unix_ms, revoked_unix_ms
    FROM tokens ORDER BY created_unix_ms, token_id";

const UPSERT_NODE: &str = "INSERT INTO nodes (node_id, name, transport, address, state, protocol_version,
        capabilities, last_heartbeat_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
    ON CONFLICT (node_id) DO UPDATE SET name = excluded.name, transport = excluded.transport,
        address = excluded.address, state = excluded.state, protocol_version = excluded.protocol_version,
        capabilities = excluded.capabilities, last_heartbeat_unix_ms = excluded.last_heartbeat_unix_ms";
const SELECT_NODE_BY_ID: &str = "SELECT node_id, name, transport, address, state, protocol_version, capabilities,
        last_heartbeat_unix_ms
    FROM nodes WHERE node_id = ?1";

// node.state_changed moves only the state: the rows themselves are kept tables (13.2). plugin.state_changed moves
// nothing: a plugin's saved state is the user's decision, written whole with UPSERT_PLUGIN (layout 6.4).
const SET_NODE_STATE: &str = "UPDATE nodes SET state = ?2 WHERE node_id = ?1";

const UPSERT_PLUGIN: &str = "INSERT INTO plugins (plugin_id, version, state, content_hash, granted_permissions, source)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT (plugin_id) DO UPDATE SET version = excluded.version, state = excluded.state,
        content_hash = excluded.content_hash, granted_permissions = excluded.granted_permissions,
        source = excluded.source";
const SELECT_PLUGIN_BY_ID: &str = "SELECT plugin_id, version, state, content_hash, granted_permissions, source
    FROM plugins WHERE plugin_id = ?1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub project_id: String,
    pub path: String,
    pub name: String,
    pub opened_unix_ms: i64,
}

/// A scoped API token. `token_hash` is the BLAKE3 of the token (`blake3:<hex>`); the token itself is never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub token_id: String,
    pub name: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub created_unix_ms: i64,
    pub revoked_unix_ms: Option<i64>,
}

/// A node. `transport` is `local` or `ssh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRow {
    pub node_id: String,
    pub name: String,
    pub transport: String,
    pub address: String,
    pub state: String,
    pub protocol_version: i64,
    pub capabilities: Vec<String>,
    pub last_heartbeat_unix_ms: Option<i64>,
}

/// An installed plugin and the permissions the user granted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub plugin_id: String,
    pub version: String,
    pub state: String,
    pub content_hash: String,
    pub granted_permissions: Vec<String>,
    pub source: String,
}

pub(crate) fn upsert_project(transaction: &SqliteTransaction<'_>, row: &ProjectRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.project_id),
        text_value(&row.path),
        text_value(&row.name),
        integer_value(row.opened_unix_ms),
    ];
    execute(transaction, UPSERT_PROJECT, &parameters)?;
    Ok(())
}

pub(crate) fn insert_token(transaction: &SqliteTransaction<'_>, row: &TokenRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.token_id),
        text_value(&row.name),
        text_value(&row.token_hash),
        word_list_value(&row.scopes)?,
        integer_value(row.created_unix_ms),
        optional_integer_value(row.revoked_unix_ms),
    ];
    execute(transaction, INSERT_TOKEN, &parameters)?;
    Ok(())
}

pub(crate) fn revoke_token(
    transaction: &SqliteTransaction<'_>,
    token_id: &str,
    revoked_unix_ms: i64,
) -> Result<(), StoreError> {
    execute(
        transaction,
        REVOKE_TOKEN,
        &[text_value(token_id), integer_value(revoked_unix_ms)],
    )?;
    Ok(())
}

pub(crate) fn upsert_node(transaction: &SqliteTransaction<'_>, row: &NodeRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.node_id),
        text_value(&row.name),
        text_value(&row.transport),
        text_value(&row.address),
        text_value(&row.state),
        integer_value(row.protocol_version),
        word_list_value(&row.capabilities)?,
        optional_integer_value(row.last_heartbeat_unix_ms),
    ];
    execute(transaction, UPSERT_NODE, &parameters)?;
    Ok(())
}

pub(crate) fn upsert_plugin(transaction: &SqliteTransaction<'_>, row: &PluginRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.plugin_id),
        text_value(&row.version),
        text_value(&row.state),
        text_value(&row.content_hash),
        word_list_value(&row.granted_permissions)?,
        text_value(&row.source),
    ];
    execute(transaction, UPSERT_PLUGIN, &parameters)?;
    Ok(())
}

pub(crate) fn set_node_state(
    transaction: &SqliteTransaction<'_>,
    node_id: &str,
    state: &str,
) -> Result<(), StoreError> {
    execute(transaction, SET_NODE_STATE, &[text_value(node_id), text_value(state)])?;
    Ok(())
}

pub fn get_project(reader: &IndexReader, project_id: &str) -> Result<Option<ProjectRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_PROJECT_BY_ID, &[text_value(project_id)])?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(ProjectRow {
        project_id: read_text(row, 0)?,
        path: read_text(row, 1)?,
        name: read_text(row, 2)?,
        opened_unix_ms: read_integer(row, 3)?,
    }))
}

/// The token with this hash, revoked or not: the caller decides what a revoked token means.
pub fn find_token_by_hash(reader: &IndexReader, token_hash: &str) -> Result<Option<TokenRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_TOKEN_BY_HASH, &[text_value(token_hash)])?;
    match rows.first() {
        Some(row) => Ok(Some(token_from_columns(row)?)),
        None => Ok(None),
    }
}

/// Every token, revoked ones included, oldest first: `token.list`, and the daemon's token cache at startup.
pub fn list_tokens(reader: &IndexReader) -> Result<Vec<TokenRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_ALL_TOKENS, &[])?;
    let mut tokens = Vec::with_capacity(rows.len());
    for row in &rows {
        tokens.push(token_from_columns(row)?);
    }
    Ok(tokens)
}

/// Columns in the order of `SELECT_TOKEN_BY_HASH` and `SELECT_ALL_TOKENS`.
fn token_from_columns(row: &[SqlValue]) -> Result<TokenRow, StoreError> {
    Ok(TokenRow {
        token_id: read_text(row, 0)?,
        name: read_text(row, 1)?,
        token_hash: read_text(row, 2)?,
        scopes: read_word_list(row, 3)?,
        created_unix_ms: read_integer(row, 4)?,
        revoked_unix_ms: read_optional_integer(row, 5)?,
    })
}

pub fn get_node(reader: &IndexReader, node_id: &str) -> Result<Option<NodeRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_NODE_BY_ID, &[text_value(node_id)])?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(NodeRow {
        node_id: read_text(row, 0)?,
        name: read_text(row, 1)?,
        transport: read_text(row, 2)?,
        address: read_text(row, 3)?,
        state: read_text(row, 4)?,
        protocol_version: read_integer(row, 5)?,
        capabilities: read_word_list(row, 6)?,
        last_heartbeat_unix_ms: read_optional_integer(row, 7)?,
    }))
}

pub fn get_plugin(reader: &IndexReader, plugin_id: &str) -> Result<Option<PluginRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_PLUGIN_BY_ID, &[text_value(plugin_id)])?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(PluginRow {
        plugin_id: read_text(row, 0)?,
        version: read_text(row, 1)?,
        state: read_text(row, 2)?,
        content_hash: read_text(row, 3)?,
        granted_permissions: read_word_list(row, 4)?,
        source: read_text(row, 5)?,
    }))
}

fn word_list_value(words: &[String]) -> Result<SqlValue, StoreError> {
    match serde_json::to_string(words) {
        Ok(json) => Ok(SqlValue::Text(json)),
        Err(error) => Err(StoreError::DatabaseWriteFailed {
            reason: format!("a list of words can't be written as JSON: {error}"),
        }),
    }
}

fn read_word_list(row: &[SqlValue], index: usize) -> Result<Vec<String>, StoreError> {
    let json = read_text(row, index)?;
    match serde_json::from_str::<Vec<String>>(&json) {
        Ok(words) => Ok(words),
        Err(error) => Err(StoreError::IndexDamaged {
            reason: format!("column {index} should hold a JSON list of text: {error}"),
        }),
    }
}
