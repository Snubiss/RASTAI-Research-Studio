//! One-time login codes and origin-bound sessions (flows/platform.md 4.1, ADR-0034 decisions 1, 4 and 10).
//!
//! - `token.login_code.create` (over daemon.sock, where the peer's user proves the owner) calls `create_login_code`.
//!   A code is 32 random bytes as 64 lowercase hex characters, single use, valid for 60 s.
//! - `POST /api/login` calls `redeem_login_code` with the request's Origin, which the gate has already required. The
//!   new session is bound to that Origin and gets a request key; the answer sets the session cookie and returns the
//!   key in its body.
//! - The gate (gate.rs) finds the session for a cookie, checks the Origin against `bound_origin` and, for anything
//!   but GET or HEAD, the request key against `key_hash`.
//!
//! - Scoped API tokens (`token.create`) live in `rastai.db`; the daemon keeps their hashes and scopes here too
//!   (`scoped_tokens`), loaded at startup and updated by `token.create`, `token.revoke` and every revoke that sweeps
//!   a chain of tokens (a disabled plugin, the start sweep; contracts 8.2), so the gate stays one plain function over
//!   this struct and a revoked token is refused on its very next request (`RRS-C1006`).
//!
//! Login codes and sessions live in memory only: a restart signs everyone out. Only hashes are kept (`hash_bytes`
//! of the hex text as sent), so a memory dump of the daemon holds no usable code, cookie or key, and comparing
//! hashes takes the same time whichever character of a guess is wrong. No HTTP types here; the login endpoint is in router.rs.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::{Mutex, MutexGuard};

use rastai_core::error::CoreError;
use rastai_core::random::random_bytes;
use rastai_core::scopes::Scope;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::hashing::hash_bytes;

use crate::auth_files::lowercase_hex;

/// How long a login code works after it is made (contracts 15.10). Long enough for a window or browser to open.
pub const LOGIN_CODE_LIFETIME_SECONDS: i64 = 60;
/// A session unused this long is gone (contracts 15.10); the next request signs in again.
pub const SESSION_IDLE_EXPIRY_HOURS: i64 = 24;
/// Login codes, session ids and session keys are all this many random bytes, as 64 lowercase hex characters.
const SECRET_BYTES: usize = 32;
/// The cookie is `rastai_session_<daemon id prefix>` (contracts 8.6): two daemons reached on two loopback ports
/// (one through an SSH tunnel) share one cookie jar, since browsers ignore ports, so the names must differ.
const COOKIE_NAME_PREFIX: &str = "rastai_session_";
/// HttpOnly keeps the cookie from scripts; SameSite=Strict keeps other sites from sending it; no Secure flag, as
/// plain-HTTP loopback would then never get it back (flows/platform.md 4.1).
const COOKIE_ATTRIBUTES: &str = "HttpOnly; SameSite=Strict; Path=/";

const MILLISECONDS_PER_SECOND: i64 = 1000;
const MILLISECONDS_PER_HOUR: i64 = 3_600_000;

/// What the daemon knows about sign-in: the owner token's hash, the cookie name, the live login codes and sessions.
/// The two lists sit behind std locks that are never held across an `.await` (layout section 8.2).
#[derive(Debug)]
pub struct AuthState {
    /// `hash_bytes` of the owner token's text (auth_files.rs).
    pub owner_token_hash: String,
    /// `rastai_session_<daemon id prefix>`.
    pub cookie_name: String,
    pub login_codes: Mutex<Vec<LoginCode>>,
    pub sessions: Mutex<Vec<Session>>,
    /// Every scoped API token, revoked ones included, as `rastai.db`'s `tokens` table holds them.
    pub scoped_tokens: Mutex<Vec<ScopedToken>>,
}

/// A scoped API token as the gate checks it: never its text, only `hash_bytes` of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedToken {
    pub token_id: String,
    pub token_hash: String,
    pub scopes: Vec<Scope>,
    pub revoked: bool,
}

/// A login code not yet redeemed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCode {
    pub code_hash: String,
    pub expires_unix_ms: i64,
}

/// A signed-in browser window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// `hash_bytes` of the cookie's value.
    pub session_id_hash: String,
    /// The Origin of the `POST /api/login` that made it: `http://127.0.0.1:47820`. Every cookie request that carries
    /// an Origin must carry this one, because browsers send the cookie to every port on the same host.
    pub bound_origin: String,
    /// `hash_bytes` of the request key's hex text, as the web app sends it in `Rastai-Session-Key`.
    pub key_hash: String,
    pub last_used_unix_ms: i64,
}

/// A fresh login code, for `token.login_code.create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLoginCode {
    /// 64 lowercase hex characters; the `#login=<code>` of the start URL.
    pub code: String,
    pub expires_unix_ms: i64,
}

/// What `POST /api/login` hands back: the cookie's value and the request key. Neither is kept in this form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    pub session_id: String,
    pub session_key: String,
}

/// An empty sign-in state for a daemon whose owner token hashes to `owner_token_hash`.
pub fn new_auth_state(owner_token_hash: String, daemon_id_prefix: &str) -> AuthState {
    AuthState {
        owner_token_hash,
        cookie_name: session_cookie_name(daemon_id_prefix),
        login_codes: Mutex::new(Vec::new()),
        sessions: Mutex::new(Vec::new()),
        scoped_tokens: Mutex::new(Vec::new()),
    }
}

pub fn session_cookie_name(daemon_id_prefix: &str) -> String {
    format!("{COOKIE_NAME_PREFIX}{daemon_id_prefix}")
}

/// A new single-use login code. Expired codes are dropped on the way, so the list stays as short as the codes
/// made in the last minute.
pub fn create_login_code(auth: &AuthState, now_unix_ms: i64) -> Result<NewLoginCode, CoreError> {
    let code = new_secret()?;
    let expires_unix_ms = now_unix_ms + LOGIN_CODE_LIFETIME_SECONDS * MILLISECONDS_PER_SECOND;
    let mut codes = lock_list(&auth.login_codes);
    codes.retain(|login_code| login_code.expires_unix_ms > now_unix_ms);
    codes.push(LoginCode {
        code_hash: hash_bytes(code.as_bytes()),
        expires_unix_ms,
    });
    Ok(NewLoginCode { code, expires_unix_ms })
}

/// Trades a login code for a session bound to `origin`. The code is used up either way it is found; an unknown,
/// used or expired code is `RRS-C1007`. Only the caller decides that the Origin is acceptable (the gate requires one
/// equal to `http://` plus Host): a request refused there never reaches this, so its code still works afterwards.
pub fn redeem_login_code(
    auth: &AuthState,
    code: &str,
    origin: &str,
    now_unix_ms: i64,
) -> Result<NewSession, Diagnostic> {
    if !take_login_code(auth, code, now_unix_ms) {
        return Err(diagnostic("RRS-C1007", &[]));
    }
    let new_session = new_session_secrets().map_err(|error| random_source_diagnostic(&error))?;
    let session = Session {
        session_id_hash: hash_bytes(new_session.session_id.as_bytes()),
        bound_origin: origin.to_string(),
        key_hash: hash_bytes(new_session.session_key.as_bytes()),
        last_used_unix_ms: now_unix_ms,
    };
    let mut sessions = lock_list(&auth.sessions);
    sessions.retain(|kept| !is_idle_too_long(kept, now_unix_ms));
    sessions.push(session);
    Ok(new_session)
}

/// The live session a cookie names, or `RRS-C1002` when there is none: never made, from before a restart, or idle
/// for 24 hours (then it is also dropped).
pub fn find_live_session(auth: &AuthState, session_id: &str, now_unix_ms: i64) -> Result<Session, Diagnostic> {
    let session_id_hash = hash_bytes(session_id.as_bytes());
    let mut sessions = lock_list(&auth.sessions);
    sessions.retain(|kept| !is_idle_too_long(kept, now_unix_ms));
    let found = sessions
        .iter()
        .find(|session| session.session_id_hash == session_id_hash);
    match found {
        Some(session) => Ok(session.clone()),
        None => Err(diagnostic("RRS-C1002", &[])),
    }
}

/// Marks a session used now. The gate calls it only after a request passed every check, so a refused request
/// never keeps a session alive.
pub fn touch_session(auth: &AuthState, session_id_hash: &str, now_unix_ms: i64) {
    let mut sessions = lock_list(&auth.sessions);
    for session in sessions.iter_mut() {
        if session.session_id_hash == session_id_hash {
            session.last_used_unix_ms = now_unix_ms;
        }
    }
}

/// Adds a token the gate accepts from now on: one read from `rastai.db` at startup, or one `token.create` just made.
/// A token already known by id is replaced, so loading it twice keeps one entry.
pub fn add_scoped_token(auth: &AuthState, token: ScopedToken) {
    let mut tokens = lock_list(&auth.scoped_tokens);
    tokens.retain(|known| known.token_id != token.token_id);
    tokens.push(token);
}

/// Forgets a token `token.create` added before its insert, when the insert failed or was refused: nobody holds its
/// text yet, and the database has no row for it (contracts 8.2, "The gate's cache follows the database").
pub fn remove_scoped_token(auth: &AuthState, token_id: &str) {
    let mut tokens = lock_list(&auth.scoped_tokens);
    tokens.retain(|known| known.token_id != token_id);
}

/// Marks a token revoked, so its next request is refused with `RRS-C1006`. An unknown id changes nothing.
pub fn revoke_scoped_token(auth: &AuthState, token_id: &str) {
    let mut tokens = lock_list(&auth.scoped_tokens);
    for token in tokens.iter_mut() {
        if token.token_id == token_id {
            token.revoked = true;
        }
    }
}

/// The scoped token whose text hashes to `token_hash`, revoked or not; the gate decides what a revoked one means.
pub fn find_scoped_token(auth: &AuthState, token_hash: &str) -> Option<ScopedToken> {
    let tokens = lock_list(&auth.scoped_tokens);
    tokens.iter().find(|token| token.token_hash == token_hash).cloned()
}

/// The `Set-Cookie` value that signs a window in.
pub fn session_cookie_header(auth: &AuthState, session_id: &str) -> String {
    format!("{}={session_id}; {COOKIE_ATTRIBUTES}", auth.cookie_name)
}

/// Removes the code if it is live and reports whether it was. A code past its expiry is removed too, and fails.
fn take_login_code(auth: &AuthState, code: &str, now_unix_ms: i64) -> bool {
    let code_hash = hash_bytes(code.as_bytes());
    let mut codes = lock_list(&auth.login_codes);
    let position = codes.iter().position(|login_code| login_code.code_hash == code_hash);
    let Some(position) = position else {
        return false;
    };
    let login_code = codes.remove(position);
    login_code.expires_unix_ms > now_unix_ms
}

fn new_session_secrets() -> Result<NewSession, CoreError> {
    Ok(NewSession {
        session_id: new_secret()?,
        session_key: new_secret()?,
    })
}

fn new_secret() -> Result<String, CoreError> {
    let bytes = random_bytes(SECRET_BYTES)?;
    Ok(lowercase_hex(&bytes))
}

fn is_idle_too_long(session: &Session, now_unix_ms: i64) -> bool {
    now_unix_ms - session.last_used_unix_ms >= SESSION_IDLE_EXPIRY_HOURS * MILLISECONDS_PER_HOUR
}

fn random_source_diagnostic(error: &CoreError) -> Diagnostic {
    let reason = match error {
        CoreError::RandomUnavailable { reason } => reason.clone(),
        other => format!("{other:?}"),
    };
    diagnostic("RRS-C0502", &[("reason", &reason)])
}

/// A thread that panicked while holding a list leaves the lock poisoned. The list itself is still whole (every
/// change above is one push, remove or retain), so sign-in carries on rather than locking the owner out.
fn lock_list<Item>(list: &Mutex<Vec<Item>>) -> MutexGuard<'_, Vec<Item>> {
    match list.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const NOW_UNIX_MS: i64 = 1_800_000_000_000;
    const ORIGIN: &str = "http://127.0.0.1:47820";

    fn auth() -> AuthState {
        new_auth_state(hash_bytes(b"owner"), "0a1b2c3d")
    }

    #[test]
    fn a_code_works_once() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        assert_eq!(code.code.len(), 64);
        assert_eq!(code.expires_unix_ms, NOW_UNIX_MS + 60_000);
        let session = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS + 1).unwrap();
        assert_eq!(session.session_id.len(), 64);
        assert_eq!(session.session_key.len(), 64);
        let again = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS + 2).unwrap_err();
        assert_eq!(again.code, "RRS-C1007");
    }

    #[test]
    fn a_code_61_seconds_old_is_refused_and_gone() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        let late = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS + 61_000).unwrap_err();
        assert_eq!(late.code, "RRS-C1007");
        assert!(lock_list(&auth.login_codes).is_empty());
    }

    #[test]
    fn an_unknown_code_is_refused() {
        let auth = auth();
        let unknown = redeem_login_code(&auth, "00", ORIGIN, NOW_UNIX_MS).unwrap_err();
        assert_eq!(unknown.code, "RRS-C1007");
    }

    #[test]
    fn only_hashes_are_kept() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        assert_eq!(
            lock_list(&auth.login_codes)[0].code_hash,
            hash_bytes(code.code.as_bytes())
        );
        let new_session = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS).unwrap();
        let session = lock_list(&auth.sessions)[0].clone();
        assert_eq!(session.session_id_hash, hash_bytes(new_session.session_id.as_bytes()));
        assert_eq!(session.key_hash, hash_bytes(new_session.session_key.as_bytes()));
        assert_eq!(session.bound_origin, ORIGIN);
    }

    #[test]
    fn a_session_idle_for_24_hours_is_gone() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        let new_session = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS).unwrap();
        let almost = NOW_UNIX_MS + 24 * 3_600_000 - 1;
        assert!(find_live_session(&auth, &new_session.session_id, almost).is_ok());
        let idle = find_live_session(&auth, &new_session.session_id, NOW_UNIX_MS + 24 * 3_600_000).unwrap_err();
        assert_eq!(idle.code, "RRS-C1002");
        assert!(lock_list(&auth.sessions).is_empty());
    }

    #[test]
    fn touching_a_session_keeps_it_alive() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        let new_session = redeem_login_code(&auth, &code.code, ORIGIN, NOW_UNIX_MS).unwrap();
        let later = NOW_UNIX_MS + 20 * 3_600_000;
        touch_session(&auth, &hash_bytes(new_session.session_id.as_bytes()), later);
        assert!(find_live_session(&auth, &new_session.session_id, later + 20 * 3_600_000).is_ok());
    }

    #[test]
    fn the_cookie_is_http_only_strict_and_named_for_the_daemon() {
        let auth = auth();
        assert_eq!(
            session_cookie_header(&auth, "abc"),
            "rastai_session_0a1b2c3d=abc; HttpOnly; SameSite=Strict; Path=/"
        );
    }

    #[test]
    fn scoped_tokens_are_added_found_and_revoked() {
        let auth = auth();
        let token = ScopedToken {
            token_id: "tok_a".to_string(),
            token_hash: hash_bytes(b"secret"),
            scopes: vec![Scope::Read],
            revoked: false,
        };
        add_scoped_token(&auth, token.clone());
        add_scoped_token(&auth, token.clone());
        assert_eq!(lock_list(&auth.scoped_tokens).len(), 1);
        assert_eq!(find_scoped_token(&auth, &hash_bytes(b"secret")), Some(token));
        assert_eq!(find_scoped_token(&auth, &hash_bytes(b"other")), None);
        revoke_scoped_token(&auth, "tok_a");
        revoke_scoped_token(&auth, "tok_unknown");
        assert!(find_scoped_token(&auth, &hash_bytes(b"secret")).unwrap().revoked);
    }
}
