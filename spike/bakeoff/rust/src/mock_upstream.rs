//! A stand-in for llama-server's streaming /completion that answers instantly, so a load test measures
//! the gateway, not the GPU. Shared by both gateways in the bake-off.

use std::convert::Infallible;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::header;
use axum::response::Response;
use axum::routing::post;
use serde::Deserialize;

const FIRST_FAKE_TOKEN_ID: u32 = 1000;

#[derive(Deserialize)]
struct CompletionRequest {
    #[serde(default)]
    n_predict: u32,
}

fn event(index: u32) -> Bytes {
    let token_id = FIRST_FAKE_TOKEN_ID + index % 1000;
    Bytes::from(format!("data: {{\"content\":\" token{index}\",\"tokens\":[{token_id}],\"stop\":false}}\n\n"))
}

async fn completion(body: Bytes) -> Response {
    let request: CompletionRequest = serde_json::from_slice(&body).unwrap_or(CompletionRequest { n_predict: 0 });
    let final_event = Bytes::from_static(b"data: {\"content\":\"\",\"tokens\":[],\"stop\":true}\n\n");
    let events = (0..request.n_predict).map(event).chain(std::iter::once(final_event));
    let stream = futures_util::stream::iter(events.map(Ok::<_, Infallible>));
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

pub fn command(arguments: &[String]) -> Result<(), String> {
    let port: u16 = crate::parse_argument(arguments, 0, "port")?;
    let router = Router::new().route("/completion", post(completion));
    crate::serve_on_loopback(port, router)
}
