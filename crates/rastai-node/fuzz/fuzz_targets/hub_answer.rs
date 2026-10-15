//! Fuzz target for the Hugging Face readers of the Hub's and the CDN's answers (contracts 12.5, ADR-0037). The bytes come
//! from the network, so arbitrary input must never panic a reader, and what a reader accepts must be safe to use:
//!
//! - the first byte picks a reader (a file list, a Content-Range value, a redirect's Location, a plain id) and the rest
//!   is its input; `rastai_node::downloads::check_hub_answer` holds the checks and returns the first broken promise;
//! - a file list the reader accepts names only safe relative files and plain commit and blob ids;
//! - a redirect the production Hub names never leads to a loopback address.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_node::downloads::check_hub_answer;

fuzz_target!(|bytes: &[u8]| {
    if let Err(problem) = check_hub_answer(bytes) {
        panic!("a Hub answer broke a promise: {problem}");
    }
});
