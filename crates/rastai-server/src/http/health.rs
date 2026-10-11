//! `GET /api/health` (contracts 3.3, layout section 11.4): public, and the first thing every client asks. The CLI
//! waits for it after starting a daemon and compares `version`; scripts read `apis` before they call anything.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rastai_core::versions::{API_PACKAGES, APP_VERSION, NODE_PROTOCOL_VERSION, PLUGIN_API_VERSION};
use serde_json::{Value, json};

/// The daemon answers health only once it is fully started, so the status is always "ok".
const STATUS_OK: &str = "ok";
const JSON_CONTENT_TYPE: &str = "application/json";
/// A cached answer would hide a daemon restarted with another version.
const NO_STORE: &str = "no-store";

/// `{"status", "version", "apis", "plugin_api", "node_protocol"}`. Fields are only ever added (section 11.1).
pub fn health_body() -> Value {
    json!({
        "status": STATUS_OK,
        "version": APP_VERSION,
        "apis": API_PACKAGES,
        "plugin_api": PLUGIN_API_VERSION,
        "node_protocol": NODE_PROTOCOL_VERSION,
    })
}

pub async fn health_endpoint() -> Response {
    let headers = [
        (header::CONTENT_TYPE, JSON_CONTENT_TYPE),
        (header::CACHE_CONTROL, NO_STORE),
    ];
    (StatusCode::OK, headers, health_body().to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_names_the_versions() {
        let body = health_body();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], APP_VERSION);
        assert_eq!(body["apis"], json!(["rastai.commands.v1", "rastai.events.v1"]));
        assert_eq!(body["plugin_api"], PLUGIN_API_VERSION);
        assert_eq!(body["node_protocol"], 1);
    }
}
