//! Tokenizer parity and speed: encode the stress corpus, compare with the expected IDs, then time
//! `repeats` passes over the whole corpus on one thread.

use std::time::Instant;

use serde_json::json;
use tokenizers::Tokenizer;

fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))
}

fn encode(tokenizer: &Tokenizer, text: &str) -> Result<Vec<u32>, String> {
    let add_special_tokens = false; // the gateway adds <bos> itself
    tokenizer
        .encode(text, add_special_tokens)
        .map(|encoding| encoding.get_ids().to_vec())
        .map_err(|error| error.to_string())
}

pub fn command(arguments: &[String]) -> Result<(), String> {
    let tokenizer_path = crate::argument(arguments, 0, "tokenizer.json")?;
    let corpus: Vec<String> = read_json(crate::argument(arguments, 1, "corpus.json")?)?;
    let expected: Vec<Vec<u32>> = read_json(crate::argument(arguments, 2, "expected.json")?)?;
    let repeats: usize = crate::parse_argument(arguments, 3, "repeats")?;

    let load_start = Instant::now();
    let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|error| error.to_string())?;
    let load_milliseconds = load_start.elapsed().as_secs_f64() * 1000.0;

    let mut mismatches = 0;
    let mut token_count = 0;
    for (text, expected_ids) in corpus.iter().zip(&expected) {
        let ids = encode(&tokenizer, text)?;
        token_count += ids.len();
        if &ids != expected_ids {
            mismatches += 1;
        }
    }

    let timing_start = Instant::now();
    for _ in 0..repeats {
        for text in &corpus {
            std::hint::black_box(encode(&tokenizer, text)?);
        }
    }
    let seconds = timing_start.elapsed().as_secs_f64();

    println!(
        "{}",
        json!({
            "cases": corpus.len(),
            "tokens": token_count,
            "mismatches": mismatches,
            "load_ms": load_milliseconds,
            "tokens_per_second": (token_count * repeats) as f64 / seconds,
        })
    );
    Ok(())
}
