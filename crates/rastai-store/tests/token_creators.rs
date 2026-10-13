//! Token creators in `rastai.db` (contracts 8.2 "Token creators and revocation", threat model R28): a revoke sweeps
//! the whole `created_by` chain under a token or a plugin in one write, with one time, and a token made by a token
//! is refused in the writer's transaction once that creator is revoked or gone.

#![allow(clippy::unwrap_used)]

use rastai_store::error::StoreError;
use rastai_store::index::kept_tables::{TokenRow, list_tokens};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, close_store, open_store};
use rastai_store::writer_thread::write_index;
use tokio::runtime::Runtime;

mod common;
use common::{remove_test_folder, test_paths};

const OWNER: &str = "owner";
const FIRST_REVOKE_MS: i64 = 100;
const SECOND_REVOKE_MS: i64 = 200;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread().build().unwrap()
}

fn token(token_id: &str, created_by: &str) -> TokenRow {
    TokenRow {
        token_id: token_id.to_string(),
        name: token_id.to_string(),
        token_hash: format!("blake3:{token_id}"),
        scopes: vec!["system.admin".to_string()],
        created_unix_ms: 1,
        revoked_unix_ms: None,
        created_by: created_by.to_string(),
    }
}

fn write(runtime: &Runtime, store: &Store, write: IndexWrite) -> Result<(), StoreError> {
    runtime.block_on(write_index(&store.index, write))
}

fn revoke_token(token_id: &str, revoked_unix_ms: i64) -> IndexWrite {
    IndexWrite::RevokeToken {
        token_id: token_id.to_string(),
        revoked_unix_ms,
    }
}

/// Each token's id and revoke time, oldest first.
fn revoke_times(store: &Store) -> Vec<(String, Option<i64>)> {
    let reader = open_index_reader(&store.index_readers).unwrap();
    let tokens = list_tokens(&reader).unwrap();
    tokens
        .into_iter()
        .map(|row| (row.token_id, row.revoked_unix_ms))
        .collect()
}

fn expected(rows: &[(&str, Option<i64>)]) -> Vec<(String, Option<i64>)> {
    rows.iter()
        .map(|(token_id, revoked_unix_ms)| (token_id.to_string(), *revoked_unix_ms))
        .collect()
}

#[test]
fn revoking_a_token_revokes_its_whole_chain_and_nothing_above_or_beside_it() {
    let paths = test_paths("token-chain");
    let store = open_store(&paths).unwrap();
    let runtime = runtime();
    // tok_a (the owner's) made tok_b, which made tok_c and tok_d; tok_e is the owner's too.
    for row in [
        token("tok_a", OWNER),
        token("tok_b", "token:tok_a"),
        token("tok_c", "token:tok_b"),
        token("tok_d", "token:tok_b"),
        token("tok_e", OWNER),
    ] {
        write(&runtime, &store, IndexWrite::InsertToken(row)).unwrap();
    }
    write(&runtime, &store, revoke_token("tok_b", FIRST_REVOKE_MS)).unwrap();
    let after_b = expected(&[
        ("tok_a", None),
        ("tok_b", Some(FIRST_REVOKE_MS)),
        ("tok_c", Some(FIRST_REVOKE_MS)),
        ("tok_d", Some(FIRST_REVOKE_MS)),
        ("tok_e", None),
    ]);
    assert_eq!(revoke_times(&store), after_b);
    // Revoking the root later keeps each swept token's first time.
    write(&runtime, &store, revoke_token("tok_a", SECOND_REVOKE_MS)).unwrap();
    let after_a = expected(&[
        ("tok_a", Some(SECOND_REVOKE_MS)),
        ("tok_b", Some(FIRST_REVOKE_MS)),
        ("tok_c", Some(FIRST_REVOKE_MS)),
        ("tok_d", Some(FIRST_REVOKE_MS)),
        ("tok_e", None),
    ]);
    assert_eq!(revoke_times(&store), after_a);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_token_whose_creator_is_revoked_or_gone_is_refused_and_not_written() {
    let paths = test_paths("token-dead-creator");
    let store = open_store(&paths).unwrap();
    let runtime = runtime();
    write(&runtime, &store, IndexWrite::InsertToken(token("tok_a", OWNER))).unwrap();
    write(&runtime, &store, IndexWrite::InsertToken(token("tok_b", "token:tok_a"))).unwrap();
    write(&runtime, &store, revoke_token("tok_a", FIRST_REVOKE_MS)).unwrap();
    for (token_id, creator) in [("tok_late", "tok_a"), ("tok_orphan", "tok_never_made")] {
        let refused = write(
            &runtime,
            &store,
            IndexWrite::InsertToken(token(token_id, &format!("token:{creator}"))),
        )
        .unwrap_err();
        assert_eq!(
            refused,
            StoreError::TokenCreatorRevoked {
                token_id: creator.to_string()
            }
        );
        assert_eq!(refused.code(), "RRS-C1006");
    }
    // In a batch, the refused insert takes the batch's other writes with it.
    let batch = IndexWrite::Batch(vec![
        IndexWrite::InsertToken(token("tok_ok", OWNER)),
        IndexWrite::InsertToken(token("tok_late_too", "token:tok_b")),
    ]);
    assert!(write(&runtime, &store, batch).is_err());
    let rows = expected(&[("tok_a", Some(FIRST_REVOKE_MS)), ("tok_b", Some(FIRST_REVOKE_MS))]);
    assert_eq!(revoke_times(&store), rows);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn revoking_a_plugins_tokens_follows_the_tokens_they_made() {
    let paths = test_paths("token-plugin-chain");
    let store = open_store(&paths).unwrap();
    let runtime = runtime();
    for row in [
        token("tok_p", "plugin:acme"),
        token("tok_q", "token:tok_p"),
        token("tok_other", "plugin:acme_other"),
        token("tok_owner", OWNER),
    ] {
        write(&runtime, &store, IndexWrite::InsertToken(row)).unwrap();
    }
    let sweep = IndexWrite::RevokePluginTokens {
        plugin_id: "acme".to_string(),
        revoked_unix_ms: FIRST_REVOKE_MS,
    };
    write(&runtime, &store, sweep).unwrap();
    let rows = expected(&[
        ("tok_other", None),
        ("tok_owner", None),
        ("tok_p", Some(FIRST_REVOKE_MS)),
        ("tok_q", Some(FIRST_REVOKE_MS)),
    ]);
    let mut found = revoke_times(&store);
    found.sort();
    assert_eq!(found, rows);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

/// A row that is already revoked admits no one, so its creator isn't checked: a rebuild copies the revoked tokens a
/// revoked or gone token made, as they were.
#[test]
fn a_revoked_token_is_written_whatever_its_creator() {
    let paths = test_paths("token-revoked-copy");
    let store = open_store(&paths).unwrap();
    let runtime = runtime();
    write(&runtime, &store, IndexWrite::InsertToken(token("tok_a", OWNER))).unwrap();
    write(&runtime, &store, revoke_token("tok_a", FIRST_REVOKE_MS)).unwrap();
    for (token_id, creator) in [("tok_child", "token:tok_a"), ("tok_orphan", "token:tok_never_made")] {
        let revoked_child = TokenRow {
            revoked_unix_ms: Some(FIRST_REVOKE_MS),
            ..token(token_id, creator)
        };
        write(&runtime, &store, IndexWrite::InsertToken(revoked_child)).unwrap();
    }
    let rows = expected(&[
        ("tok_a", Some(FIRST_REVOKE_MS)),
        ("tok_child", Some(FIRST_REVOKE_MS)),
        ("tok_orphan", Some(FIRST_REVOKE_MS)),
    ]);
    assert_eq!(revoke_times(&store), rows);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}
