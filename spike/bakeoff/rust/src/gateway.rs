//! Minimal OpenAI-compatible gateway (spec v5 §18), the part of the backend every model call goes through.
//!
//! POST /v1/chat/completions (streaming):
//!   1. render the Gemma chat template and tokenize it here: the gateway owns tokenization;
//!   2. send the token IDs upstream to llama-server's /completion, streaming;
//!   3. relay each upstream event to the client as an OpenAI "chat.completion.chunk";
//!   4. log the prompt and generated token IDs to SQLite, then finish with "data: [DONE]".
//! GET /health answers {"status":"ok"} once the server is ready.

use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;
use tokenizers::Tokenizer;
use tokio::sync::{mpsc, oneshot};

const BEGIN_TEXT_ID: u32 = 2;
const DEFAULT_MAX_TOKENS: u32 = 64;
const CHUNK_CHANNEL_CAPACITY: usize = 64;
const INSERT_LOG_ROW: &str =
    "INSERT INTO gateway_log (request_id, prompt_ids, generated_ids, created_unix_ms) VALUES (?1, ?2, ?3, ?4)";

struct Gateway {
    tokenizer: Tokenizer,
    upstream_completion_url: String,
    client: reqwest::Client,
    log_writer: std_mpsc::Sender<LogRecord>,
    next_request_number: AtomicU64,
}

#[derive(Deserialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// One event from llama-server's streaming /completion.
#[derive(Deserialize)]
struct UpstreamEvent {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tokens: Vec<u32>,
    #[serde(default)]
    stop: bool,
}

// ---- Prompt ------------------------------------------------------------------------------------------

fn render_chat_template(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for message in messages {
        let role = if message.role == "assistant" { "model" } else { message.role.as_str() };
        prompt.push_str(&format!("<start_of_turn>{role}\n{}<end_of_turn>\n", message.content));
    }
    prompt.push_str("<start_of_turn>model\n");
    prompt
}

fn prompt_token_ids(tokenizer: &Tokenizer, messages: &[ChatMessage]) -> Result<Vec<u32>, String> {
    let encoding = tokenizer.encode(render_chat_template(messages), false).map_err(|error| error.to_string())?;
    let mut ids = vec![BEGIN_TEXT_ID];
    ids.extend_from_slice(encoding.get_ids());
    Ok(ids)
}

// ---- Streaming -----------------------------------------------------------------------------------------

fn openai_chunk(request_id: &str, content: Option<&str>) -> Bytes {
    let (delta, finish_reason) = match content {
        Some(text) => (json!({"content": text}), serde_json::Value::Null),
        None => (json!({}), json!("stop")),
    };
    let chunk = json!({
        "id": request_id,
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    });
    Bytes::from(format!("data: {chunk}\n\n"))
}

/// Splits a byte stream into lines, keeping any incomplete last line for the next call.
fn take_complete_lines(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(newline) = buffer.iter().position(|&byte| byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=newline).collect();
        lines.push(String::from_utf8_lossy(&line).trim_end().to_string());
    }
    lines
}

async fn relay_upstream(gateway: Arc<Gateway>, request_id: String, prompt_ids: Vec<u32>, max_tokens: u32, sender: mpsc::Sender<Bytes>) {
    let body = json!({
        "prompt": prompt_ids, "n_predict": max_tokens, "stream": true, "return_tokens": true,
        "cache_prompt": false, "temperature": 0.0, "top_k": 1,
    });
    let response = gateway
        .client
        .post(&gateway.upstream_completion_url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await;
    let Ok(response) = response else { return };

    let mut generated_ids = Vec::new();
    let mut buffer = Vec::new();
    let mut upstream = response.bytes_stream();
    'reading: while let Some(Ok(bytes)) = upstream.next().await {
        buffer.extend_from_slice(&bytes);
        for line in take_complete_lines(&mut buffer) {
            let Some(payload) = line.strip_prefix("data: ") else { continue };
            let Ok(event) = serde_json::from_str::<UpstreamEvent>(payload) else { continue };
            generated_ids.extend_from_slice(&event.tokens);
            if !event.content.is_empty() && sender.send(openai_chunk(&request_id, Some(&event.content))).await.is_err() {
                return; // the client went away
            }
            if event.stop {
                break 'reading;
            }
        }
    }

    log_token_ids(gateway, request_id.clone(), prompt_ids, generated_ids).await;
    let _ = sender.send(openai_chunk(&request_id, None)).await;
    let _ = sender.send(Bytes::from_static(b"data: [DONE]\n\n")).await;
}

// ---- Logging ---------------------------------------------------------------------------------------------
//
// One thread owns the SQLite connection and writes rows in arrival order. Requests hand it a record and wait
// (asynchronously) for the commit. The obvious alternative, a Mutex<Connection> locked from tokio's
// blocking pool, convoys at high concurrency: hundreds of OS threads queue on one lock.

struct LogRecord {
    request_id: String,
    prompt_ids: Vec<u32>,
    generated_ids: Vec<u32>,
    committed: oneshot::Sender<()>,
}

fn start_log_writer(connection: Connection) -> std_mpsc::Sender<LogRecord> {
    let (sender, receiver) = std_mpsc::channel::<LogRecord>();
    std::thread::spawn(move || {
        let mut insert = connection.prepare(INSERT_LOG_ROW).expect("could not prepare the log insert");
        for record in receiver {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
            let row = (record.request_id, json!(record.prompt_ids).to_string(), json!(record.generated_ids).to_string(), now);
            if let Err(error) = insert.execute(row) {
                eprintln!("could not log a request: {error}");
            }
            let _ = record.committed.send(());
        }
    });
    sender
}

fn open_log(path: &str) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS gateway_log (
                 request_id TEXT PRIMARY KEY, prompt_ids TEXT NOT NULL,
                 generated_ids TEXT NOT NULL, created_unix_ms INTEGER NOT NULL);",
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

async fn log_token_ids(gateway: Arc<Gateway>, request_id: String, prompt_ids: Vec<u32>, generated_ids: Vec<u32>) {
    let (committed, commit_finished) = oneshot::channel();
    let record = LogRecord { request_id, prompt_ids, generated_ids, committed };
    if gateway.log_writer.send(record).is_ok() {
        let _ = commit_finished.await;
    }
}

// ---- HTTP ------------------------------------------------------------------------------------------------

async fn chat_completions(State(gateway): State<Arc<Gateway>>, body: Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<ChatRequest>(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid chat request").into_response();
    };
    let prompt_ids = match prompt_token_ids(&gateway.tokenizer, &request.messages) {
        Ok(ids) => ids,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let request_id = format!("chatcmpl-{}", gateway.next_request_number.fetch_add(1, Ordering::Relaxed));

    let (sender, receiver) = mpsc::channel(CHUNK_CHANNEL_CAPACITY);
    tokio::spawn(relay_upstream(gateway.clone(), request_id, prompt_ids, request.max_tokens, sender));
    let chunks = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (Ok::<_, Infallible>(chunk), receiver))
    });

    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(chunks))
        .unwrap()
}

async fn health() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], r#"{"status":"ok"}"#)
}

pub fn command(arguments: &[String]) -> Result<(), String> {
    let port: u16 = crate::parse_argument(arguments, 0, "port")?;
    let upstream_url = crate::argument(arguments, 1, "upstream-url")?;
    let tokenizer = Tokenizer::from_file(crate::argument(arguments, 2, "tokenizer.json")?).map_err(|error| error.to_string())?;
    let log = open_log(crate::argument(arguments, 3, "log.sqlite")?)?;

    let gateway = Arc::new(Gateway {
        tokenizer,
        upstream_completion_url: format!("{upstream_url}/completion"),
        client: reqwest::Client::new(),
        log_writer: start_log_writer(log),
        next_request_number: AtomicU64::new(0),
    });
    let router = Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(gateway);

    crate::serve_on_loopback(port, router)
}
