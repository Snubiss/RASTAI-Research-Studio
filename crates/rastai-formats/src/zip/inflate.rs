//! DEFLATE for zip entries, the one file that imports `miniz_oxide` (contracts 2.5, ADR-0037 decision 1). The zip
//! container around it (central directory, limits, zip slip) is in-house, in `reader.rs`; this file only turns the
//! compressed bytes of one entry into the bytes it declares.
//!
//! The reader feeds the stream one chunk of compressed bytes at a time through `inflate_chunk`, which writes the
//! output as it goes, so memory doesn't grow with the entry. Output stops at the entry's declared uncompressed size: a
//! stream that holds more is refused before the extra bytes reach the sink, so a lying header can't make this inflate
//! past its cap.

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

use crate::byte_source::ByteSink;
use crate::error::FormatsError;

/// Decompressed bytes produced per call into miniz_oxide.
const OUTPUT_CHUNK_BYTES: usize = 32 * 1024;
/// Once the entry has no output left, the stream is still asked for one byte: a byte it produces is more output than
/// the entry declares, and the stream's end is still seen when it produces none.
const PROBE_BYTES: usize = 1;

/// The name of the cap in `RRS-E5002` when the stream holds more output than its entry declares.
const DECLARED_SIZE_CAP: &str = "declared size";

const REASON_CORRUPT: &str = "the deflate stream is corrupt or cut short";
const REASON_NO_PROGRESS: &str = "the deflate stream stopped making progress before its end";

/// The state of one entry's inflate: the miniz_oxide stream, how much output it has made and the size the entry
/// declares. The caller keeps the compressed bytes and feeds them in with `inflate_chunk`.
pub struct InflateStream {
    state: Box<InflateState>,
    output: Vec<u8>,
    produced: u64,
    declared_uncompressed_bytes: u64,
}

/// What one `inflate_chunk` call did: how many input bytes the stream took, how many output bytes it wrote, and
/// whether the deflate stream has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflateProgress {
    pub consumed: usize,
    pub written: usize,
    pub finished: bool,
}

/// A new stream for one entry that declares `declared_uncompressed_bytes` of output.
pub fn new_inflate_stream(declared_uncompressed_bytes: u64) -> InflateStream {
    InflateStream {
        state: InflateState::new_boxed(DataFormat::Raw),
        output: vec![0u8; OUTPUT_CHUNK_BYTES],
        produced: 0,
        declared_uncompressed_bytes,
    }
}

/// One call into miniz_oxide with the next compressed bytes. The stream writes its output to `sink` as it goes. Output
/// past the declared size is `RRS-E5002` and is never written. A call that takes no input and makes no output, before
/// the stream has ended, is `RRS-E5007`: the compressed bytes ran out first, so the caller has no way to make progress.
/// Whether the stream produced the declared size, and whether every compressed byte was used, is the caller's to check.
pub fn inflate_chunk(
    stream: &mut InflateStream,
    input: &[u8],
    sink: &mut dyn ByteSink,
) -> Result<InflateProgress, FormatsError> {
    let remaining = stream.declared_uncompressed_bytes - stream.produced;
    let wanted = output_wanted(remaining);
    let result = inflate(&mut stream.state, input, &mut stream.output[..wanted], MZFlush::None);
    let consumed = result.bytes_consumed;
    let written = result.bytes_written;
    if written as u64 > remaining {
        return Err(FormatsError::ZipLimit {
            what: DECLARED_SIZE_CAP,
            size: stream.produced + written as u64,
            limit: stream.declared_uncompressed_bytes,
        });
    }
    sink.write_bytes(&stream.output[..written])?;
    stream.produced += written as u64;
    let finished = match result.status {
        Ok(MZStatus::StreamEnd) => true,
        Ok(_) if consumed == 0 && written == 0 => return Err(malformed(REASON_NO_PROGRESS)),
        Ok(_) => false,
        Err(_) => return Err(malformed(REASON_CORRUPT)),
    };
    Ok(InflateProgress {
        consumed,
        written,
        finished,
    })
}

/// How much output one call may write: the rest of the declared size, at most one chunk. Once the declared size is
/// reached, one byte of probe room is enough to catch output past it.
fn output_wanted(remaining: u64) -> usize {
    if remaining == 0 {
        return PROBE_BYTES;
    }
    remaining.min(OUTPUT_CHUNK_BYTES as u64) as usize
}

fn malformed(reason: &'static str) -> FormatsError {
    FormatsError::ZipMalformed { reason }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::byte_source::VecSink;

    /// `hello hello hello hello`, raw DEFLATE (no zlib header), from Python:
    /// `zlib.compressobj(9, zlib.DEFLATED, -15)`.
    const DEFLATED_HELLO: &[u8] = &[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01];
    const HELLO: &[u8] = b"hello hello hello hello";

    /// Feeds the whole stream to `inflate_chunk` the way the reader does, one call after another, until it ends or
    /// refuses. Returns the outcome and the bytes the sink received.
    fn inflate_all(compressed: &[u8], declared: u64) -> (Result<(), FormatsError>, Vec<u8>) {
        let mut stream = new_inflate_stream(declared);
        let mut sink = VecSink { bytes: Vec::new() };
        let mut at = 0;
        let result = loop {
            match inflate_chunk(&mut stream, &compressed[at..], &mut sink) {
                Ok(progress) => {
                    at += progress.consumed;
                    if progress.finished {
                        break Ok(());
                    }
                }
                Err(error) => break Err(error),
            }
        };
        (result, sink.bytes)
    }

    #[test]
    fn inflates_a_stream_to_its_declared_size() {
        let (result, bytes) = inflate_all(DEFLATED_HELLO, HELLO.len() as u64);
        result.unwrap();
        assert_eq!(bytes, HELLO);
    }

    #[test]
    fn a_declared_size_smaller_than_the_stream_is_refused_before_the_extra_bytes() {
        // The stream has more than the declared 5 bytes. The sink gets at most the declared 5, and the refusal is the
        // declared-size cap of RRS-E5002, not a damaged stream.
        let (result, bytes) = inflate_all(DEFLATED_HELLO, 5);
        assert_eq!(result.unwrap_err().code(), "RRS-E5002");
        assert!(bytes.len() <= 5, "{} bytes past the declared 5", bytes.len());
    }

    #[test]
    fn a_corrupt_stream_is_refused() {
        let mut broken = DEFLATED_HELLO.to_vec();
        broken[0] = 0xff;
        broken[1] = 0xff;
        let (result, _) = inflate_all(&broken, HELLO.len() as u64);
        assert_eq!(result.unwrap_err().code(), "RRS-E5007");
    }

    #[test]
    fn a_truncated_stream_is_refused_when_its_input_runs_out() {
        let (result, _) = inflate_all(&DEFLATED_HELLO[..DEFLATED_HELLO.len() - 3], HELLO.len() as u64);
        assert_eq!(result.unwrap_err().code(), "RRS-E5007");
    }

    #[test]
    fn an_empty_entry_is_one_empty_final_block() {
        // BFINAL=1, BTYPE=01 (fixed Huffman), then the end-of-block code: zero bytes out.
        let (result, bytes) = inflate_all(&[0x03, 0x00], 0);
        result.unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn a_chunk_that_ends_the_stream_reports_it_finished() {
        let mut stream = new_inflate_stream(HELLO.len() as u64);
        let mut sink = VecSink { bytes: Vec::new() };
        let progress = inflate_chunk(&mut stream, DEFLATED_HELLO, &mut sink).unwrap();
        assert!(progress.finished);
        assert_eq!(progress.written, HELLO.len());
        assert_eq!(progress.consumed, DEFLATED_HELLO.len());
    }
}
