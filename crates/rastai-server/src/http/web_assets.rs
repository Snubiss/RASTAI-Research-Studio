//! The web app: `/` and the files of `apps/web/dist/`, embedded by build.rs (layout rust-control-plane.md 8.1; frontend
//! 8.8). Public, as the gate says: the page is code and carries no secret, and every API call the page makes is still
//! gated. Its headers are the ones the spec asks for: the index is `no-store`, hashed assets are `immutable`, and every
//! response carries the content security policy below.

use axum::body::Bytes;
use axum::http::{HeaderName, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// One embedded file: its URL path, its type and its bytes. The table itself is written by build.rs.
pub struct WebAsset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/web_assets_table.rs"));

/// The page's address and the file it is served from.
const ROOT_PATH: &str = "/";
const INDEX_PATH: &str = "/index.html";
/// The build names its hashed files under this folder; a new build is a new name, so they never change.
const HASHED_PATH_PREFIX: &str = "/assets/";
const NO_STORE: &str = "no-store";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NOSNIFF: &str = "nosniff";
/// Scripts and styles from this origin only. `wasm-unsafe-eval` lets the formats validator compile its own bytes, and
/// frames come only from this origin, where plugin UIs live under /plugins/. Nothing comes from another origin.
/// The one inline style is the locked focus ring of the security UI (@rastai/ui LOCKED_FOCUS_RING_CSS), allowed by its
/// hash: the security UI's ring must show whatever the theme, so it can't wait for a stylesheet. A test in apps/web
/// keeps the hash in step with that CSS.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self' 'sha256-KBjDeAaGjMz7k+ImE4bb6dA8AiFaIHw8lSI/QlRGHqU='; img-src 'self' data:; font-src 'self'; \
    connect-src 'self'; frame-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; \
    frame-ancestors 'none'";

/// GET and HEAD for an embedded file; any other method or path is not found.
pub(crate) async fn web_assets_endpoint(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    match find_asset(uri.path()) {
        Some(asset) => asset_response(asset, method == Method::HEAD),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The embedded file at this path. "/" is the index page.
fn find_asset(path: &str) -> Option<&'static WebAsset> {
    let wanted = if path == ROOT_PATH { INDEX_PATH } else { path };
    WEB_ASSETS.iter().find(|asset| asset.path == wanted)
}

/// The response for one file. A HEAD request gets the same headers and no body.
fn asset_response(asset: &'static WebAsset, head_only: bool) -> Response {
    let cache_control = if asset.path.starts_with(HASHED_PATH_PREFIX) {
        IMMUTABLE
    } else {
        NO_STORE
    };
    let headers: [(HeaderName, &str); 4] = [
        (header::CONTENT_TYPE, asset.content_type),
        (header::CACHE_CONTROL, cache_control),
        (header::CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY),
        (header::X_CONTENT_TYPE_OPTIONS, NOSNIFF),
    ];
    let body = if head_only {
        Bytes::new()
    } else {
        Bytes::from_static(asset.bytes)
    };
    (StatusCode::OK, headers, body).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_index_page() {
        let index = find_asset(INDEX_PATH).unwrap();
        assert!(find_asset(ROOT_PATH).is_some_and(|asset| std::ptr::eq(asset, index)));
        assert_eq!(index.content_type, "text/html; charset=utf-8");
    }

    #[test]
    fn a_path_nothing_embeds_is_absent() {
        assert!(find_asset("/no/such/file.js").is_none());
    }
}
