//! The request gate (flows/platform.md 4.2, ADR-0034, layout section 8.3a): one plain function that runs first on
//! every HTTP request and every daemon.sock connection, and the header reading in front of it.
//!
//! Over loopback TCP, in order:
//! 1. **Host** must be `127.0.0.1`, `localhost` or `[::1]`, on any port (an SSH tunnel forwards another port).
//!    Anything else is `RRS-C1003`: a page that points `attacker.test` at 127.0.0.1 still sends `Host: attacker.test`
//!    (DNS rebinding).
//! 2. **Origin**, when present, must be `http://` plus Host; `null` (a sandboxed iframe) never is (`RRS-C1004`).
//! 3. Public paths (health, plugin UI assets, the web app) pass with no principal.
//! 4. `POST /api/login` must carry an Origin: the session it makes is bound to it.
//! 5. A **session cookie** must name a live session, come from the Origin that redeemed it, and, for anything but
//!    GET or HEAD, carry the session's request key. Without an Origin only GET or HEAD with `Sec-Fetch-Site`
//!    `same-origin` or `none` pass. A **bearer token** must be the owner token (scoped tokens come with
//!    `token.create`, M1-27). No credential is `RRS-C1002`.
//!
//! A header that is malformed (not visible ASCII) or sent twice when it may only be sent once is refused with the
//! code of the rule it would have fed, never guessed at, so an odd request can't slip between two readings of it.
//! The `request_gate` fuzz target runs `read_request_head` and the gate on arbitrary bytes.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use rastai_core::principal::{Principal, owner_principal};
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::hashing::hash_bytes;
use rastai_transport::peer::{PeerIdentity, check_same_user};

use crate::http::sessions::{AuthState, Session, find_live_session, touch_session};

/// Any port: SSH tunnels forward other ports (plan §10 row 22). Lowercase only, as browsers send them; `LOCALHOST`
/// would also fail the Origin comparison, so it is refused here already.
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "localhost", "[::1]"];
pub const LOGIN_PATH: &str = "/api/login";
pub const HEALTH_PATH: &str = "/api/health";
/// Plugin UI assets are static files with no secrets (contracts 3.3); the Host rule still applies.
const PLUGIN_ASSETS_PREFIX: &str = "/plugins/";
/// Paths that are never public: the HTTP API, Connect services, the gateway and MCP. Every other plain path is the
/// web app.
const PROTECTED_PATH_PREFIXES: &[&str] = &["/api/", "/rastai.", "/v1", "/mcp"];
/// A path with these is never public, so `/plugins/../api/blobs` can't reach the API through the public rule.
const UNPLAIN_PATH_PARTS: &[&str] = &["//", "..", "%", "\\"];
const SAFE_METHODS_WITHOUT_ORIGIN: &[&str] = &["GET", "HEAD"];
const SEC_FETCH_SITES_WITHOUT_ORIGIN: &[&str] = &["same-origin", "none"];
/// The cookie alone reads only static files and blobs (ADR-0034 decision 10).
const METHODS_WITHOUT_SESSION_KEY: &[&str] = &["GET", "HEAD"];
const NULL_ORIGIN: &str = "null";
const ORIGIN_SCHEME: &str = "http://";
const BEARER_SCHEME: &str = "bearer";
/// How a refusal names a header that wasn't sent (the login endpoint's own Origin check uses it too).
pub const MISSING_HEADER_TEXT: &str = "(none)";
const HOST_PORT_SEPARATOR: char = ':';
const IPV6_HOST_END: char = ']';
const IPV6_HOST_START: char = '[';
/// A port is 1 to 65535: at most five digits.
const MAX_PORT_DIGITS: usize = 5;
const COOKIE_SEPARATOR: char = ';';
const COOKIE_VALUE_SEPARATOR: char = '=';

// Header names as HTTP/1.1 servers hand them over: lowercase.
const HOST_HEADER: &str = "host";
const ORIGIN_HEADER: &str = "origin";
const SEC_FETCH_SITE_HEADER: &str = "sec-fetch-site";
const COOKIE_HEADER: &str = "cookie";
const SESSION_KEY_HEADER: &str = "rastai-session-key";
const AUTHORIZATION_HEADER: &str = "authorization";

/// What the gate reads of a request. Plain text, already taken apart, so the gate itself never parses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: String,
    pub path: String,
    /// Empty when the request had no Host header.
    pub host: String,
    pub origin: Option<String>,
    pub sec_fetch_site: Option<String>,
    /// The value of this daemon's `rastai_session_<prefix>` cookie.
    pub cookie_session_id: Option<String>,
    /// The `Rastai-Session-Key` header.
    pub session_key: Option<String>,
    pub bearer_token: Option<String>,
}

/// Which listener a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listener {
    LoopbackTcp,
    /// daemon.sock; the kernel reported the connecting process's user.
    DaemonSocket {
        peer_uid: u32,
    },
}

/// Reads the gate's view of a request from its method, path and headers (`(lowercase name, value bytes)`). A Host
/// or Origin that isn't visible ASCII or is sent twice is refused (`RRS-C1003`, `RRS-C1004`), and so is a
/// credential header that is (`RRS-C1002`).
pub fn read_request_head(
    method: &str,
    path: &str,
    headers: &[(&str, &[u8])],
    cookie_name: &str,
) -> Result<RequestHead, Diagnostic> {
    let host = single_header(headers, HOST_HEADER).map_err(|values| host_refusal(&values))?;
    let origin = single_header(headers, ORIGIN_HEADER).map_err(|values| origin_refusal(&values))?;
    let sec_fetch_site = single_header(headers, SEC_FETCH_SITE_HEADER).map_err(|values| origin_refusal(&values))?;
    let session_key = single_header(headers, SESSION_KEY_HEADER).map_err(|_| credential_refusal())?;
    let authorization = single_header(headers, AUTHORIZATION_HEADER).map_err(|_| credential_refusal())?;
    Ok(RequestHead {
        method: method.to_string(),
        path: path.to_string(),
        host: host.unwrap_or_default(),
        origin,
        sec_fetch_site,
        cookie_session_id: find_session_cookie(headers, cookie_name)?,
        session_key,
        bearer_token: authorization.as_deref().and_then(bearer_token_of),
    })
}

/// The gate. `Ok(None)` lets a public path through with no principal; `Ok(Some)` names who is calling.
pub fn check_request_gate(
    request: &RequestHead,
    listener: &Listener,
    auth: &AuthState,
) -> Result<Option<Principal>, Diagnostic> {
    if let Listener::DaemonSocket { peer_uid } = listener {
        return check_socket_peer(*peer_uid);
    }
    check_loopback_host(&request.host)?;
    check_origin_matches_host(request)?;
    if is_public_path(&request.path) {
        return Ok(None);
    }
    if request.path == LOGIN_PATH {
        return require_origin_for_login(request).map(|_| None);
    }
    match (&request.cookie_session_id, &request.bearer_token) {
        (Some(session_id), _) => check_session(request, session_id, auth),
        (None, Some(token)) => check_bearer(token, auth),
        (None, None) => Err(credential_refusal()),
    }
}

/// daemon.sock serves its own user only; the folder is 0700 and `accept_local` already refused anyone else, so this
/// is the second lock on the same door. A peer that isn't the owner is an unknown credential.
fn check_socket_peer(peer_uid: u32) -> Result<Option<Principal>, Diagnostic> {
    let peer = PeerIdentity {
        user_id: peer_uid,
        process_id: None,
    };
    match check_same_user(&peer) {
        Ok(()) => Ok(Some(owner_principal())),
        Err(_) => Err(credential_refusal()),
    }
}

fn check_loopback_host(host: &str) -> Result<(), Diagnostic> {
    let (name, port) = split_host_and_port(host);
    let port_is_valid = port.is_none_or(is_valid_port);
    if LOOPBACK_HOSTS.contains(&name) && port_is_valid {
        return Ok(());
    }
    Err(host_refusal(host))
}

/// `127.0.0.1:47820` gives `("127.0.0.1", Some("47820"))`, `[::1]:9000` gives `("[::1]", Some("9000"))`, `localhost`
/// gives `("localhost", None)`. Anything else comes back whole as the name, which then matches no loopback name.
fn split_host_and_port(host: &str) -> (&str, Option<&str>) {
    if host.starts_with(IPV6_HOST_START) {
        let Some(end) = host.find(IPV6_HOST_END) else {
            return (host, None);
        };
        let (name, rest) = host.split_at(end + IPV6_HOST_END.len_utf8());
        return match rest.strip_prefix(HOST_PORT_SEPARATOR) {
            Some(port) => (name, Some(port)),
            None if rest.is_empty() => (name, None),
            None => (host, None),
        };
    }
    match host.split_once(HOST_PORT_SEPARATOR) {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    }
}

fn is_valid_port(port: &str) -> bool {
    let only_digits =
        !port.is_empty() && port.len() <= MAX_PORT_DIGITS && port.bytes().all(|byte| byte.is_ascii_digit());
    only_digits && port.parse::<u16>().is_ok_and(|number| number > 0)
}

/// When present, Origin must be the page's own: `http://` plus the Host the request was sent to.
fn check_origin_matches_host(request: &RequestHead) -> Result<(), Diagnostic> {
    let Some(origin) = &request.origin else {
        return Ok(());
    };
    if origin == NULL_ORIGIN {
        return Err(origin_refusal(origin));
    }
    if *origin != format!("{ORIGIN_SCHEME}{}", request.host) {
        return Err(origin_refusal(origin));
    }
    Ok(())
}

/// Public paths need no credential. A path with `..`, `//`, `%` or `\` is never public, so a crafted path can't
/// reach the API through the public rule however a later handler would read it.
fn is_public_path(path: &str) -> bool {
    let is_plain = path.starts_with('/') && !UNPLAIN_PATH_PARTS.iter().any(|part| path.contains(part));
    if !is_plain {
        return false;
    }
    if path == HEALTH_PATH || path.starts_with(PLUGIN_ASSETS_PREFIX) {
        return true;
    }
    !PROTECTED_PATH_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

/// The session will record this Origin, so a login without one is refused, and its code is left unused.
fn require_origin_for_login(request: &RequestHead) -> Result<(), Diagnostic> {
    match &request.origin {
        Some(_) => Ok(()),
        None => Err(origin_refusal(MISSING_HEADER_TEXT)),
    }
}

fn check_session(request: &RequestHead, session_id: &str, auth: &AuthState) -> Result<Option<Principal>, Diagnostic> {
    let now_unix_ms = now_unix_ms();
    let session = find_live_session(auth, session_id, now_unix_ms)?;
    check_session_origin(request, &session)?;
    if !METHODS_WITHOUT_SESSION_KEY.contains(&request.method.as_str()) {
        check_session_key(request, &session)?;
    }
    touch_session(auth, &session.session_id_hash, now_unix_ms);
    Ok(Some(owner_principal()))
}

/// A cookie request must come from the origin that redeemed the login code. Browsers send cookies to every port on
/// 127.0.0.1, so without this a page on another loopback port could forward requests with the owner's cookie.
fn check_session_origin(request: &RequestHead, session: &Session) -> Result<(), Diagnostic> {
    match &request.origin {
        Some(origin) if *origin == session.bound_origin => Ok(()),
        Some(origin) => Err(origin_refusal(origin)),
        None if allowed_without_origin(request) => Ok(()),
        None => Err(origin_refusal(MISSING_HEADER_TEXT)),
    }
}

/// Origin and Sec-Fetch-Site are enforced only by browsers. A non-browser process on another loopback port that
/// receives the cookie can forge both, but it never sees the key, which the web app keeps in memory only.
fn check_session_key(request: &RequestHead, session: &Session) -> Result<(), Diagnostic> {
    match &request.session_key {
        // The key travels as 64 lowercase hex characters; the session stored hash_bytes of that same text at login.
        Some(key) if hash_bytes(key.as_bytes()) == session.key_hash => Ok(()),
        _ => Err(credential_refusal()),
    }
}

/// GET or HEAD with Sec-Fetch-Site same-origin or none: the app's own blob downloads and images, a typed URL.
/// An <img> on a page from another loopback port arrives as same-site and is refused.
fn allowed_without_origin(request: &RequestHead) -> bool {
    let safe_method = SAFE_METHODS_WITHOUT_ORIGIN.contains(&request.method.as_str());
    let safe_site = request
        .sec_fetch_site
        .as_deref()
        .is_some_and(|site| SEC_FETCH_SITES_WITHOUT_ORIGIN.contains(&site));
    safe_method && safe_site
}

/// The owner token, compared by hash. Scoped tokens, looked up by hash in the `tokens` table, arrive with
/// `token.create` (M1-27); until then no other token can exist, so any other is unknown.
fn check_bearer(token: &str, auth: &AuthState) -> Result<Option<Principal>, Diagnostic> {
    if hash_bytes(token.as_bytes()) == auth.owner_token_hash {
        return Ok(Some(owner_principal()));
    }
    Err(credential_refusal())
}

// ---- Reading headers ----

/// The value of a header that may be sent at most once, trimmed. `Err` carries the values as text for the refusal:
/// the header came twice, or holds a byte that isn't visible ASCII or a space.
fn single_header(headers: &[(&str, &[u8])], name: &str) -> Result<Option<String>, String> {
    let values: Vec<&[u8]> = headers
        .iter()
        .filter(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
        .collect();
    match values.as_slice() {
        [] => Ok(None),
        [value] if is_visible_text(value) => Ok(Some(String::from_utf8_lossy(value).trim().to_string())),
        _ => Err(values_as_text(&values)),
    }
}

fn is_visible_text(value: &[u8]) -> bool {
    value.iter().all(|byte| *byte == b' ' || byte.is_ascii_graphic())
}

fn values_as_text(values: &[&[u8]]) -> String {
    let texts: Vec<String> = values
        .iter()
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect();
    texts.join(", ")
}

/// This daemon's session cookie among all the `Cookie` headers' pairs. Other cookies are skipped, whatever bytes
/// they hold. The same name with two different values is refused (`RRS-C1002`): a page on another loopback port
/// can set a cookie of that name, and the gate never guesses which one is ours.
fn find_session_cookie(headers: &[(&str, &[u8])], cookie_name: &str) -> Result<Option<String>, Diagnostic> {
    let mut found: Option<String> = None;
    for (header_name, value) in headers {
        if !header_name.eq_ignore_ascii_case(COOKIE_HEADER) {
            continue;
        }
        let text = String::from_utf8_lossy(value);
        for pair in text.split(COOKIE_SEPARATOR) {
            let Some((name, cookie_value)) = pair.split_once(COOKIE_VALUE_SEPARATOR) else {
                continue;
            };
            if name.trim() != cookie_name {
                continue;
            }
            let cookie_value = cookie_value.trim().to_string();
            if found.as_ref().is_some_and(|earlier| *earlier != cookie_value) {
                return Err(credential_refusal());
            }
            found = Some(cookie_value);
        }
    }
    Ok(found)
}

/// `Bearer <token>`, the scheme in any case (RFC 9110 11.1). Any other scheme, or a token with a space in it, is no
/// bearer credential.
fn bearer_token_of(authorization: &str) -> Option<String> {
    let (scheme, token) = authorization.split_once(' ')?;
    let token = token.trim();
    let is_bearer = scheme.eq_ignore_ascii_case(BEARER_SCHEME);
    if is_bearer && !token.is_empty() && !token.contains(' ') {
        Some(token.to_string())
    } else {
        None
    }
}

// ---- Refusals ----

fn host_refusal(host: &str) -> Diagnostic {
    let host = if host.is_empty() { MISSING_HEADER_TEXT } else { host };
    diagnostic("RRS-C1003", &[("host", host)])
}

fn origin_refusal(origin: &str) -> Diagnostic {
    diagnostic("RRS-C1004", &[("origin", origin)])
}

fn credential_refusal() -> Diagnostic {
    diagnostic("RRS-C1002", &[])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port_split() {
        assert_eq!(split_host_and_port("127.0.0.1:47820"), ("127.0.0.1", Some("47820")));
        assert_eq!(split_host_and_port("localhost"), ("localhost", None));
        assert_eq!(split_host_and_port("[::1]:9000"), ("[::1]", Some("9000")));
        assert_eq!(split_host_and_port("[::1]"), ("[::1]", None));
        assert_eq!(split_host_and_port("[::1]x"), ("[::1]x", None));
        assert_eq!(split_host_and_port("[::1"), ("[::1", None));
    }

    #[test]
    fn loopback_hosts_on_any_port_pass() {
        for host in [
            "127.0.0.1:47820",
            "127.0.0.1:9000",
            "localhost:47900",
            "[::1]:1",
            "localhost",
            "[::1]",
        ] {
            assert!(check_loopback_host(host).is_ok(), "{host}");
        }
    }

    #[test]
    fn other_hosts_and_bad_ports_are_refused() {
        let refused = [
            "",
            "attacker.test",
            "attacker.test:47820",
            "127.0.0.1.nip.io:47820",
            "LOCALHOST:47820",
            "localhost.:47820",
            "127.0.0.1:",
            "127.0.0.1:0",
            "127.0.0.1:65536",
            "127.0.0.1:+80",
            "127.0.0.1:080000",
            "127.0.0.1:80:80",
            "0.0.0.0:47820",
            "[::1]x:47820",
            "user@127.0.0.1:47820",
        ];
        for host in refused {
            let refusal = check_loopback_host(host).unwrap_err();
            assert_eq!(refusal.code, "RRS-C1003", "{host}");
        }
    }

    #[test]
    fn public_paths() {
        for path in [
            "/",
            "/index.html",
            "/assets/app.js",
            "/api/health",
            "/plugins/eval/1.0.0/ui/index.html",
        ] {
            assert!(is_public_path(path), "{path}");
        }
        let not_public = [
            "/api/login",
            "/api/blobs/blake3:00",
            "/rastai.commands.v1.CommandService/Execute",
            "/v1/chat/completions",
            "/v1",
            "/mcp",
            "/plugins/../api/blobs/x",
            "//api/blobs/x",
            "/assets/%2e%2e/api",
            "/assets\\..\\api",
            "",
            "api/health",
        ];
        for path in not_public {
            assert!(!is_public_path(path), "{path}");
        }
    }

    #[test]
    fn bearer_tokens_are_read_in_any_scheme_case() {
        assert_eq!(bearer_token_of("Bearer abc"), Some("abc".to_string()));
        assert_eq!(bearer_token_of("bearer  abc "), Some("abc".to_string()));
        assert_eq!(bearer_token_of("Basic abc"), None);
        assert_eq!(bearer_token_of("Bearer"), None);
        assert_eq!(bearer_token_of("Bearer "), None);
        assert_eq!(bearer_token_of("Bearer a b"), None);
    }
}
