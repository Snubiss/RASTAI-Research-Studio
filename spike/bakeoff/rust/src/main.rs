//! Rust side of the backend bake-off (spike experiment 07). The Java side implements the same
//! command-line contract, so the experiment can run both and compare.
//!
//! Commands:
//!   safetensors-read <file>                         read and checksum every tensor, print JSON
//!   safetensors-validate <folder>                   accept or reject every file in a folder, print JSON
//!   tokenize-bench <tokenizer.json> <corpus.json> <expected.json> <repeats>
//!   gateway <port> <upstream-url> <tokenizer.json> <log.sqlite>
//!   mock-upstream <port>                            a fast stand-in for llama-server (shared tool)
//!   load <url> <concurrency> <seconds> <max-tokens> a streaming load generator (shared tool)

mod gateway;
mod load;
mod mock_upstream;
mod safetensors;
mod tokenize_bench;

use std::process::ExitCode;

const USAGE: &str = "usage: bakeoff-rust <safetensors-read|safetensors-validate|tokenize-bench|gateway|mock-upstream|load> ...";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = arguments.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &arguments[1..];

    let outcome = match command.as_str() {
        "safetensors-read" => safetensors::read_command(rest),
        "safetensors-validate" => safetensors::validate_command(rest),
        "tokenize-bench" => tokenize_bench::command(rest),
        "gateway" => gateway::command(rest),
        "mock-upstream" => mock_upstream::command(rest),
        "load" => load::command(rest),
        _ => Err(format!("unknown command {command}\n{USAGE}")),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The argument at `index`, or an error naming what was expected there.
pub fn argument<'a>(arguments: &'a [String], index: usize, name: &str) -> Result<&'a str, String> {
    arguments.get(index).map(String::as_str).ok_or_else(|| format!("missing argument <{name}>"))
}

const LOOPBACK: &str = "127.0.0.1";

/// Serves a router on a loopback port with TCP_NODELAY on every connection. Without it, streaming small
/// chunks stalls ~40 ms per response, where Nagle's algorithm meets the client's delayed ACKs.
pub fn serve_on_loopback(port: u16, router: axum::Router) -> Result<(), String> {
    use axum::serve::ListenerExt;
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((LOOPBACK, port))
            .await
            .map_err(|error| error.to_string())?
            .tap_io(|stream: &mut tokio::net::TcpStream| {
                let _ = stream.set_nodelay(true);
            });
        axum::serve(listener, router).await.map_err(|error| error.to_string())
    })
}

pub fn parse_argument<T: std::str::FromStr>(arguments: &[String], index: usize, name: &str) -> Result<T, String> {
    argument(arguments, index, name)?.parse().map_err(|_| format!("<{name}> is not a valid number"))
}
