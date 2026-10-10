//! A streaming load generator for the gateways. `concurrency` clients each send chat requests back to
//! back for `seconds`, read every chunk through to "data: [DONE]", and record time to first chunk and
//! total time. Shared by both gateways in the bake-off.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::json;

const DONE_MARKER: &[u8] = b"data: [DONE]";
// A request that hasn't finished by then counts as timed out, so a stalled server shows up in the
// results instead of hanging the load test.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPT: &str = "Explain in two sentences why the sky is blue.";

#[derive(Default)]
struct ClientRecord {
    first_chunk_microseconds: Vec<u64>,
    total_microseconds: Vec<u64>,
    errors: u64,
    timeouts: u64,
}

fn percentile_milliseconds(values: &mut [u64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable();
    let index = ((values.len() as f64 * fraction) as usize).min(values.len() - 1);
    values[index] as f64 / 1000.0
}

/// One request, read to the end. Returns (time to first chunk, total time), or None on any failure.
async fn one_request(client: &reqwest::Client, url: &str, body: &str) -> Option<(Duration, Duration)> {
    let start = Instant::now();
    let response = client.post(url).header("Content-Type", "application/json").body(body.to_string()).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut first_chunk = None;
    let mut tail = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        first_chunk.get_or_insert_with(|| start.elapsed());
        tail.extend_from_slice(&chunk);
        if tail.len() > 256 {
            tail.drain(..tail.len() - 256); // only the end matters: it must close with [DONE]
        }
    }
    let finished = tail.windows(DONE_MARKER.len()).any(|window| window == DONE_MARKER);
    finished.then(|| (first_chunk.unwrap_or_default(), start.elapsed()))
}

async fn client_loop(client: reqwest::Client, url: Arc<String>, body: Arc<String>, deadline: Instant) -> ClientRecord {
    let mut record = ClientRecord::default();
    while Instant::now() < deadline {
        match tokio::time::timeout(REQUEST_TIMEOUT, one_request(&client, &url, &body)).await {
            Ok(Some((first, total))) => {
                record.first_chunk_microseconds.push(first.as_micros() as u64);
                record.total_microseconds.push(total.as_micros() as u64);
            }
            Ok(None) => record.errors += 1,
            Err(_) => record.timeouts += 1,
        }
    }
    record
}

pub fn command(arguments: &[String]) -> Result<(), String> {
    let url = crate::argument(arguments, 0, "url")?.to_string();
    let concurrency: usize = crate::parse_argument(arguments, 1, "concurrency")?;
    let seconds: f64 = crate::parse_argument(arguments, 2, "seconds")?;
    let max_tokens: u32 = crate::parse_argument(arguments, 3, "max-tokens")?;
    let body = json!({"messages": [{"role": "user", "content": PROMPT}], "max_tokens": max_tokens, "stream": true}).to_string();

    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    let records: Vec<ClientRecord> = runtime.block_on(async move {
        let client = reqwest::Client::builder().pool_max_idle_per_host(concurrency).build().unwrap();
        let (url, body) = (Arc::new(url), Arc::new(body));
        let deadline = Instant::now() + Duration::from_secs_f64(seconds);
        let tasks: Vec<_> = (0..concurrency)
            .map(|_| tokio::spawn(client_loop(client.clone(), url.clone(), body.clone(), deadline)))
            .collect();
        let mut records = Vec::new();
        for task in tasks {
            records.push(task.await.unwrap());
        }
        records
    });

    let mut first_chunk: Vec<u64> = records.iter().flat_map(|record| record.first_chunk_microseconds.clone()).collect();
    let mut total: Vec<u64> = records.iter().flat_map(|record| record.total_microseconds.clone()).collect();
    let errors: u64 = records.iter().map(|record| record.errors).sum();
    let timeouts: u64 = records.iter().map(|record| record.timeouts).sum();
    println!(
        "{}",
        json!({
            "concurrency": concurrency,
            "requests": total.len(),
            "errors": errors,
            "timeouts": timeouts,
            "requests_per_second": total.len() as f64 / seconds,
            "first_chunk_ms_p50": percentile_milliseconds(&mut first_chunk, 0.50),
            "first_chunk_ms_p99": percentile_milliseconds(&mut first_chunk, 0.99),
            "total_ms_p50": percentile_milliseconds(&mut total, 0.50),
            "total_ms_p99": percentile_milliseconds(&mut total, 0.99),
        })
    );
    Ok(())
}
