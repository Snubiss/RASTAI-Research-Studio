//! Byte access without paths (layout 4.3). Readers take a `ByteSource` and writers a `ByteSink`, so the same code
//! reads a file the caller opened, a zip entry or a browser buffer, and never opens anything itself.
//!
//! Both traits are small and synchronous, with two implementations each: an in-memory one (WASM, tests, small
//! files) and a file one (native; it also compiles for WASM, where the browser never constructs it).

use std::fs::File;
use std::io::{ErrorKind, Write};

use crate::error::FormatsError;
use crate::limits::ImportLimits;

const READ_PAST_END: &str = "read past the end of the source";
const LENGTH_UNKNOWN: &str = "the source reported no bytes but has some; its size couldn't be read";

/// Bytes that can be read at any offset. `read_at` fills the whole buffer or fails; there are no short reads.
pub trait ByteSource {
    fn length(&self) -> u64;
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), FormatsError>;
}

/// The write side: archives stream to a sink instead of building in memory.
pub trait ByteSink {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), FormatsError>;
}

/// Bytes already in memory.
pub struct SliceSource<'a> {
    pub bytes: &'a [u8],
}

/// A file the caller opened. Native only in practice; the browser has no files.
pub struct FileSource {
    pub file: File,
}

/// Collects written bytes in memory.
pub struct VecSink {
    pub bytes: Vec<u8>,
}

/// A file the caller created.
pub struct FileSink {
    pub file: File,
}

impl ByteSource for SliceSource<'_> {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), FormatsError> {
        let Some(range) = slice_range(self.bytes.len(), offset, buffer.len()) else {
            return Err(read_failed(offset, buffer.len(), READ_PAST_END.to_string()));
        };
        buffer.copy_from_slice(&self.bytes[range]);
        Ok(())
    }
}

impl ByteSource for FileSource {
    /// A file whose size can't be read reports 0, because `length` has no error (layout 4.3). `read_all_bytes`
    /// checks every 0 with a one-byte read, so such a file fails with `RRS-D3001` instead of reading as empty. The
    /// operating system's reason for the size failure is lost; the trait has nowhere to carry it.
    fn length(&self) -> u64 {
        match self.file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(_) => 0,
        }
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), FormatsError> {
        read_file_at(&self.file, offset, buffer).map_err(|error| read_failed(offset, buffer.len(), error.to_string()))
    }
}

impl ByteSink for VecSink {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), FormatsError> {
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

impl ByteSink for FileSink {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), FormatsError> {
        self.file.write_all(bytes).map_err(write_failed)
    }
}

/// Reads a whole source into memory, after checking its length against `max_file_bytes`, so a huge file is
/// refused before a buffer is allocated for it (`RRS-E5003`).
pub fn read_all_bytes(source: &dyn ByteSource, limits: &ImportLimits) -> Result<Vec<u8>, FormatsError> {
    let size_bytes = source.length();
    let too_large = FormatsError::FileTooLarge {
        size_bytes,
        limit_bytes: limits.max_file_bytes,
    };
    if size_bytes > limits.max_file_bytes {
        return Err(too_large);
    }
    // On a 32-bit target (WASM) a length can pass the limit and still not fit in memory.
    let Ok(buffer_length) = usize::try_from(size_bytes) else {
        return Err(too_large);
    };
    if buffer_length == 0 {
        check_really_empty(source)?;
    }
    let mut buffer = vec![0u8; buffer_length];
    source.read_at(0, &mut buffer)?;
    Ok(buffer)
}

/// A source that says it has no bytes may only have failed to read its size (`FileSource::length`). A one-byte
/// read tells the two apart: it fails on a really empty source and succeeds on the other.
fn check_really_empty(source: &dyn ByteSource) -> Result<(), FormatsError> {
    let mut probe = [0u8; 1];
    if source.read_at(0, &mut probe).is_ok() {
        return Err(read_failed(0, 0, LENGTH_UNKNOWN.to_string()));
    }
    Ok(())
}

/// The byte range `offset .. offset + length` inside a slice of `total` bytes, or None when it doesn't fit.
fn slice_range(total: usize, offset: u64, length: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(length)?;
    if end > total {
        return None;
    }
    Some(start..end)
}

/// A full disk keeps its own code (`RRS-S3002`), because its fix differs: free space, then retry.
fn write_failed(error: std::io::Error) -> FormatsError {
    let reason = error.to_string();
    if error.kind() == ErrorKind::StorageFull {
        FormatsError::DiskFull { reason }
    } else {
        FormatsError::WriteFailed { reason }
    }
}

fn read_failed(offset: u64, length: usize, reason: String) -> FormatsError {
    FormatsError::ReadFailed {
        offset,
        length: length as u64,
        reason,
    }
}

/// Positioned read on Unix: it leaves the file's cursor alone, so a shared `FileSource` stays correct.
#[cfg(unix)]
fn read_file_at(file: &File, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

/// Elsewhere (Windows, WASM): seek, then read. Correct as long as one caller reads a `FileSource` at a time.
#[cfg(not(unix))]
fn read_file_at(file: &File, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut reader = file;
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(buffer)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn slice_source_reads_inside_and_refuses_past_the_end() {
        let source = SliceSource { bytes: b"hello world" };
        let mut buffer = [0u8; 5];
        source.read_at(6, &mut buffer).unwrap();
        assert_eq!(&buffer, b"world");
        assert_eq!(source.length(), 11);
        let error = source.read_at(7, &mut buffer).unwrap_err();
        assert_eq!(error.code(), "RRS-D3001");
        // An offset that would overflow is refused, not wrapped.
        assert!(source.read_at(u64::MAX, &mut buffer).is_err());
    }

    #[test]
    fn read_all_bytes_checks_the_limit_before_reading() {
        let source = SliceSource { bytes: b"0123456789" };
        let small = ImportLimits {
            max_file_bytes: 4,
            ..ImportLimits::default()
        };
        let error = read_all_bytes(&source, &small).unwrap_err();
        assert_eq!(
            error,
            FormatsError::FileTooLarge {
                size_bytes: 10,
                limit_bytes: 4
            }
        );
        assert_eq!(error.code(), "RRS-E5003");
        assert_eq!(
            read_all_bytes(&source, &ImportLimits::default()).unwrap(),
            b"0123456789"
        );
    }

    #[test]
    fn file_source_and_sink_round_trip() {
        let path = std::env::temp_dir().join(format!("rastai-formats-byte-source-{}.bin", std::process::id()));
        let mut sink = FileSink {
            file: File::create(&path).unwrap(),
        };
        sink.write_bytes(b"abc").unwrap();
        sink.write_bytes(b"def").unwrap();
        drop(sink);
        let source = FileSource {
            file: File::open(&path).unwrap(),
        };
        assert_eq!(source.length(), 6);
        let mut buffer = [0u8; 3];
        source.read_at(2, &mut buffer).unwrap();
        assert_eq!(&buffer, b"cde");
        assert_eq!(source.read_at(5, &mut buffer).unwrap_err().code(), "RRS-D3001");
        std::fs::remove_file(&path).unwrap();
    }

    // /dev/full fails every write with ENOSPC; a file opened read-only fails with a different error.
    #[cfg(target_os = "linux")]
    #[test]
    fn file_sink_tells_a_full_disk_from_other_failures() {
        let mut full = FileSink {
            file: File::options().write(true).open("/dev/full").unwrap(),
        };
        let error = full.write_bytes(b"x").unwrap_err();
        assert!(matches!(error, FormatsError::DiskFull { .. }), "{error:?}");
        assert_eq!(error.code(), "RRS-S3002");
        let path = std::env::temp_dir().join(format!("rastai-formats-read-only-sink-{}.bin", std::process::id()));
        File::create(&path).unwrap();
        let mut read_only = FileSink {
            file: File::open(&path).unwrap(),
        };
        let error = read_only.write_bytes(b"x").unwrap_err();
        assert!(matches!(error, FormatsError::WriteFailed { .. }), "{error:?}");
        assert_eq!(error.code(), "RRS-S3005");
        std::fs::remove_file(&path).unwrap();
    }

    /// Reports 0 as its length, like a `FileSource` whose size couldn't be read, but holds bytes.
    struct LengthlessSource;

    impl ByteSource for LengthlessSource {
        fn length(&self) -> u64 {
            0
        }

        fn read_at(&self, _offset: u64, buffer: &mut [u8]) -> Result<(), FormatsError> {
            buffer.fill(b'x');
            Ok(())
        }
    }

    #[test]
    fn a_length_of_zero_is_checked_before_reading_as_empty() {
        let error = read_all_bytes(&LengthlessSource, &ImportLimits::default()).unwrap_err();
        assert_eq!(error.code(), "RRS-D3001");
        assert_eq!(
            error,
            FormatsError::ReadFailed {
                offset: 0,
                length: 0,
                reason: LENGTH_UNKNOWN.to_string()
            }
        );
        // A source that really is empty still reads as empty.
        let empty = SliceSource { bytes: b"" };
        assert_eq!(read_all_bytes(&empty, &ImportLimits::default()).unwrap(), b"");
    }

    #[test]
    fn vec_sink_collects_in_order() {
        let mut sink = VecSink { bytes: Vec::new() };
        sink.write_bytes(b"ab").unwrap();
        sink.write_bytes(b"").unwrap();
        sink.write_bytes(b"c").unwrap();
        assert_eq!(sink.bytes, b"abc");
    }
}
