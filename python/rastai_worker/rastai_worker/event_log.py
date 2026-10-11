"""The run's event log, written by the job (contracts 7.1; python layout 7.5): `runs/<run_id>/events.log`.

The file is the 8-byte header `RRSEVLG1`, then one frame per durable event:

    body_length: u32 little-endian | checksum: u32 little-endian | body: body_length bytes

The body is one `rastai.events.v1.EventRecord` and the checksum is `zlib.crc32` of it, the same CRC-32/IEEE as the
Rust reader's. The bytes are the Rust writer's too (`rastai-store/src/event_log/frame.rs`): the body is encoded by
`event_payloads.encode_message`, which writes prost's form, so `fixtures/event-logs/` holds one set of files that
both languages read and write.

`EventLogWriter` is the log's one writer:
- **One writer.** It takes an exclusive `flock` on the file before reading or writing anything. A second writer (a
  relaunch racing a live job) fails with `RRS-J3006` instead of interleaving frames.
- **Torn-tail recovery.** On open it reads every frame and truncates a torn tail (`RRS-S2001`, info; the caller
  reports it from `truncated_tail_bytes`). A corrupt frame (`RRS-S2002`) or a sequence gap (`RRS-S2003`) before
  the end is an error and the file is left alone: appending after it would hide it.
- **One write per frame**, so a crash leaves at most one torn frame. A failed write cuts the partial frame off again
  before the error returns: `RRS-S3002` on a full disk, `RRS-S3005` for any other reason.
- **fsync policy.** State changes, checkpoints, diagnostics and artifacts fsync at once (`fsync_now`); metrics are
  batched and fsynced at least once a second, on the next append or `flush_if_due`.
- **Disk-full reserve.** While it is open, `events.log.reserve` holds `EVENT_LOG_RESERVE_BYTES` of zeros, so that
  freeing it on a full disk makes room for the records that pause the run (platform flow 6.1; pause_for_full_disk
  arrives with the job process, M1-40). `close` deletes it.

How the open tells a torn tail from corruption is the Rust reader's rule (contracts 7.1, `reader.rs`), ported here
line for line so both sides give every file the same verdict. A crash only damages the end of a file, so a bad
frame is the torn tail when nothing but zeros follows its start or, for a checksum mismatch, when nothing but zeros
follows its declared end. A frame whose declared end lies past the end of the file, or a checksum mismatch with only
zeros after it, is the torn tail unless a whole, checksum-valid, version-1 frame with a later sequence lies in the
1 MiB after its header: then its length was damaged and it is corruption. When the bytes after the header start as
the record the frame should hold, a later frame may start only where one of that record's top-level fields ends (a
frame inside a field is the record's own text quoting one); otherwise every offset is a candidate. The search runs
at most 4 MiB of candidate bodies through CRC-32, so crafted bytes can't make an open slow.
"""

import errno
import os
import struct
import time
import zlib
from dataclasses import dataclass
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_J3006, RRS_S0006, RRS_S2002, RRS_S2003, RRS_S3002, RRS_S3005
from rastai_proto.common.v1.blob_pb2 import BlobRef
from rastai_proto.common.v1.states_pb2 import EVENT_CLASS_DURABLE
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.events.v1.payloads_pb2 import EventPayload

from rastai_worker.errors import WorkerError
from rastai_worker.event_payloads import decode_event_record, encode_message

# Metrics are batched and fsynced at least this often (contracts 7.1 and 15.10; the Rust writer's batched policy).
METRICS_FSYNC_INTERVAL_SECONDS = 1.0
# Freed on a full disk so the records that pause the run still fit (contracts 7.1 and 15.10; platform flow 6.1).
EVENT_LOG_RESERVE_BYTES = 1024 * 1024
# The reserve sits beside the log: runs/<run_id>/events.log.reserve (contracts 8.4).
RESERVE_FILE_SUFFIX = ".reserve"

# The first 8 bytes of every event log.
EVENT_LOG_HEADER = b"RRSEVLG1"
# body_length and checksum, the two little-endian u32s in front of every body.
FRAME_HEADER_FORMAT = "<II"
FRAME_HEADER_BYTES = 8
# The largest body the u32 length field can describe.
MAX_BODY_LENGTH = 0xFFFF_FFFF
# The record layout written and read here (EventRecord.format_version); any other version is refused, not guessed.
EVENT_RECORD_FORMAT_VERSION = 1
# A log's first record has sequence 1, and each next one adds 1.
FIRST_SEQUENCE = 1
# Permission bits for a new log, before the umask: the same as the Rust writer's (std's default for a new file).
NEW_FILE_MODE = 0o666

# What reading one frame can find: the four answers of the Rust reader's FrameRead.
FRAME_RECORD = "record"
FRAME_END_OF_FILE = "end_of_file"
FRAME_INCOMPLETE = "incomplete"
FRAME_CORRUPT = "corrupt"

# How much of the file the zero-tail check reads at a time.
ZERO_SCAN_CHUNK_BYTES = 64 * 1024
# How far past a bad frame's header the search for a whole later frame looks. A record carries hashes, not data, so
# a real frame is a few KiB: 1 MiB holds the damaged frame's true body and the next frame many times over.
LATER_FRAME_SEARCH_BYTES = 1024 * 1024
# How many candidate body bytes one search may run through CRC-32 in all. The window alone doesn't bound the time:
# bytes crafted so a length fits at every offset would cost about 1 MiB times 1 MiB. Once it runs out, the bad
# frame counts as the torn tail.
LATER_FRAME_CRC_BUDGET_BYTES = 4 * 1024 * 1024

# What trying one candidate offset in that search found.
CANDIDATE_LATER_FRAME = "later_frame"
CANDIDATE_NOTHING = "nothing"
CANDIDATE_BUDGET_SPENT = "budget_spent"

# The protobuf wire types (the low 3 bits of a field key) the field walk steps over. Groups (3 and 4) are never
# written by our encoders, so the walk stops at them.
WIRE_TYPE_VARINT = 0
WIRE_TYPE_FIXED64 = 1
WIRE_TYPE_LENGTH_DELIMITED = 2
WIRE_TYPE_FIXED32 = 5
WIRE_TYPE_BITS = 3
FIXED64_BYTES = 8
FIXED32_BYTES = 4
# A u64 varint takes at most 10 bytes, 7 bits each; the Rust walk keeps only the low 64 bits.
MAX_VARINT_BYTES = 10
UINT64_MASK = (1 << 64) - 1
# EventRecord's first two field numbers. Every encoder writes fields in number order, and both are always written,
# because a record's version (1) and sequence (1 or more) are never 0.
FORMAT_VERSION_FIELD = 1
SEQUENCE_FIELD = 2


# ---- The writer ----


class EventLogWriter:
    """The one open writer of a run's events.log. Opening it creates the log if it is missing, or recovers it after
    a crash and continues from the last good sequence plus 1."""

    def __init__(self, path: Path, run_id: str, job_id: str) -> None:
        self.path = Path(path)
        self.run_id = run_id
        self.job_id = job_id
        self.reserve_path = reserve_path_for(self.path)
        created, self.file_descriptor = open_log_file(self.path)
        try:
            lock_exclusively(self.file_descriptor, self.path)
            self.end_offset, last_sequence, self.truncated_tail_bytes = recover_log(self.file_descriptor, self.path)
            if created:
                sync_parent_folder(self.path)
            write_reserve_file(self.reserve_path)
        except BaseException:
            # Closing the file also releases the lock, if this writer got it.
            os.close(self.file_descriptor)
            raise
        self.next_sequence = last_sequence + 1
        self.has_unsynced_bytes = False
        self.last_sync_seconds = time.monotonic()
        self.is_closed = False

    def append(self, event_type: str, payload: EventPayload, blob_hashes: list[str], fsync_now: bool) -> int:
        """Appends one durable event as the log's next frame and returns its sequence. The writer numbers the
        records, so no caller can break the 1, 2, 3 ... rule."""
        if self.is_closed:
            raise ValueError(f"the event log {self.path} is closed")
        record = EventRecord(
            format_version=EVENT_RECORD_FORMAT_VERSION,
            sequence=self.next_sequence,
            run_id=self.run_id,
            job_id=self.job_id,
            emitted_unix_ms=current_unix_ms(),
            event_type=event_type,
            event_class=EVENT_CLASS_DURABLE,
            payload=payload,
            blobs=[BlobRef(hash=blob_hash) for blob_hash in blob_hashes],
        )
        self.write_frame_at_end(encode_frame(record))
        self.next_sequence += 1
        if fsync_now or batched_sync_is_due(self.last_sync_seconds, time.monotonic()):
            self.sync()
        return record.sequence

    def flush_if_due(self) -> None:
        """fsyncs batched appends once the last fsync is a second old. A job that may go quiet calls it on a timer,
        so its last metrics don't wait for the next append."""
        if self.has_unsynced_bytes and batched_sync_is_due(self.last_sync_seconds, time.monotonic()):
            self.sync()

    def sync(self) -> None:
        """fsyncs whatever was appended since the last fsync."""
        if self.has_unsynced_bytes:
            try:
                os.fsync(self.file_descriptor)
            except OSError as error:
                raise write_error(self.path, error) from error
            self.has_unsynced_bytes = False
        self.last_sync_seconds = time.monotonic()

    def close(self) -> None:
        """fsyncs, releases the lock and deletes the reserve. Closing twice does nothing."""
        if self.is_closed:
            return
        self.is_closed = True
        try:
            self.sync()
        finally:
            os.close(self.file_descriptor)
            self.reserve_path.unlink(missing_ok=True)

    def write_frame_at_end(self, frame: bytes) -> None:
        """One write at the end of the last whole frame. On failure the file is cut back to that point, so a failed
        append never leaves a torn frame for the next reader."""
        try:
            write_all_at(self.file_descriptor, frame, self.end_offset)
        except OSError as error:
            cut_back_after_failed_write(self.file_descriptor, self.end_offset)
            raise write_error(self.path, error) from error
        self.end_offset += len(frame)
        self.has_unsynced_bytes = True


def current_unix_ms() -> int:
    """The writer's clock for `emitted_unix_ms` (tests replace it to write fixed bytes)."""
    return time.time_ns() // 1_000_000


def encode_frame(record: EventRecord) -> bytes:
    """One whole frame for `record`: header, then the encoded body. A body too large for the u32 length field is
    refused rather than written, because a wrong length would make every later frame unreadable."""
    body = encode_message(record)
    if len(body) > MAX_BODY_LENGTH:
        raise WorkerError(RRS_S3005, "the record is larger than a frame's 4 GiB length field allows")
    return struct.pack(FRAME_HEADER_FORMAT, len(body), zlib.crc32(body)) + body


def batched_sync_is_due(last_sync_seconds: float, now_seconds: float) -> bool:
    return now_seconds - last_sync_seconds >= METRICS_FSYNC_INTERVAL_SECONDS


def reserve_path_for(log_path: Path) -> Path:
    return log_path.with_name(log_path.name + RESERVE_FILE_SUFFIX)


# ---- Opening, locking and recovering ----


def open_log_file(path: Path) -> tuple[bool, int]:
    """Opens the log for reading and writing, creating it when it is missing. Returns whether it was created and
    the file descriptor. A file that can't be created is a failed write; one that exists but can't be opened is a
    failed read, as in the Rust store."""
    try:
        return True, os.open(path, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, NEW_FILE_MODE)
    except FileExistsError:
        pass
    except OSError as error:
        raise write_error(path, error) from error
    try:
        return False, os.open(path, os.O_RDWR | os.O_CLOEXEC)
    except OSError as error:
        raise read_error(path, error) from error


def lock_exclusively(file_descriptor: int, path: Path) -> None:
    """Takes the exclusive flock without waiting: a second writer fails at once instead of interleaving frames."""
    # fcntl exists only on POSIX. It is imported here, not at the top, so the package still imports on Windows,
    # where python/conftest.py imports every package before the portable tests run; the worker itself is
    # POSIX-only until tier 2.
    import fcntl

    try:
        fcntl.flock(file_descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        raise WorkerError(RRS_J3006, f"another process is writing the event log {path}") from error
    except OSError as error:
        raise read_error(path, error) from error


def recover_log(file_descriptor: int, path: Path) -> tuple[int, int, int]:
    """Reads the whole log and cuts a torn tail off. Returns where the next frame goes, the last sequence in the log
    (0 when there are no records) and how many bytes were cut off."""
    valid_bytes, last_sequence = find_end_of_whole_frames(file_descriptor, path)
    file_length = file_size(file_descriptor, path)
    if valid_bytes < file_length:
        cut_file_to(file_descriptor, path, valid_bytes)
    end_offset = valid_bytes
    if valid_bytes == 0:
        # A new log, or one whose writer died before its header was whole: start the file again.
        write_header(file_descriptor, path)
        end_offset = len(EVENT_LOG_HEADER)
    return end_offset, last_sequence, file_length - valid_bytes


def write_header(file_descriptor: int, path: Path) -> None:
    try:
        write_all_at(file_descriptor, EVENT_LOG_HEADER, 0)
        os.fsync(file_descriptor)
    except OSError as error:
        raise write_error(path, error) from error


def cut_file_to(file_descriptor: int, path: Path, length: int) -> None:
    """Truncates and fsyncs, so the cut is durable before anything new is appended after it."""
    try:
        os.ftruncate(file_descriptor, length)
        os.fsync(file_descriptor)
    except OSError as error:
        raise write_error(path, error) from error


def cut_back_after_failed_write(file_descriptor: int, length: int) -> None:
    # Best effort: if even the cut fails, the next writer's open cuts the torn frame instead.
    try:
        os.ftruncate(file_descriptor, length)
    except OSError:
        pass


def write_reserve_file(reserve_path: Path) -> None:
    """Writes the reserve as real zeros and fsyncs it, so the space is taken on disk, not just promised."""
    try:
        with open(reserve_path, "wb") as reserve:
            reserve.write(bytes(EVENT_LOG_RESERVE_BYTES))
            reserve.flush()
            os.fsync(reserve.fileno())
    except OSError as error:
        reserve_path.unlink(missing_ok=True)
        raise write_error(reserve_path, error) from error


def sync_parent_folder(path: Path) -> None:
    """fsyncs the folder that holds a new log, so its directory entry survives a power cut too."""
    try:
        folder_descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(folder_descriptor)
        finally:
            os.close(folder_descriptor)
    except OSError as error:
        raise write_error(path.parent, error) from error


# ---- Reading the log back (the Rust reader's rules) ----


@dataclass(frozen=True)
class LogFile:
    """An open log being read: the writer holds the lock, so its length doesn't change while it is read."""

    file_descriptor: int
    path: Path
    length: int


@dataclass(frozen=True)
class FrameRead:
    """What reading one frame found: one of the FRAME_ kinds. `offset` is where the frame ends for a record, where
    the whole frames end for an incomplete tail, and where the bad frame starts for a corrupt one."""

    kind: str
    offset: int
    record: EventRecord | None = None


def find_end_of_whole_frames(file_descriptor: int, path: Path) -> tuple[int, int]:
    """Where the log's whole frames end and the last sequence in it (0 when there are no records). A corrupt frame
    is `RRS-S2002` and a sequence gap `RRS-S2003`; the file is never changed here."""
    log = LogFile(file_descriptor, path, file_size(file_descriptor, path))
    header_problem = check_file_header(log)
    if header_problem == FRAME_INCOMPLETE:
        return 0, 0
    if header_problem == FRAME_CORRUPT:
        raise corrupt_log_error(path, 0)
    offset = len(EVENT_LOG_HEADER)
    next_sequence = FIRST_SEQUENCE
    while True:
        frame = read_next_frame(log, offset, next_sequence)
        if frame.kind == FRAME_END_OF_FILE:
            return offset, next_sequence - 1
        if frame.kind == FRAME_INCOMPLETE:
            return frame.offset, next_sequence - 1
        if frame.kind == FRAME_CORRUPT:
            raise corrupt_log_error(path, frame.offset)
        if frame.record.sequence != next_sequence:
            raise WorkerError(
                RRS_S2003,
                f"the event log {path} skips from sequence {next_sequence - 1} to {frame.record.sequence}",
            )
        offset = frame.offset
        next_sequence += 1


def check_file_header(log: LogFile) -> str | None:
    """None when the 8-byte header is there. A header the writer hasn't finished (a prefix of RRSEVLG1, or zeros)
    is incomplete; anything else isn't an event log."""
    header = read_at(log, 0, len(EVENT_LOG_HEADER))
    if header != EVENT_LOG_HEADER[: len(header)]:
        if rest_of_file_is_zero(log, 0):
            return FRAME_INCOMPLETE
        return FRAME_CORRUPT
    if len(header) < len(EVENT_LOG_HEADER):
        return FRAME_INCOMPLETE
    return None


def read_next_frame(log: LogFile, frame_start: int, next_sequence: int) -> FrameRead:
    header_bytes = read_at(log, frame_start, FRAME_HEADER_BYTES)
    if not header_bytes:
        return FrameRead(FRAME_END_OF_FILE, frame_start)
    if len(header_bytes) < FRAME_HEADER_BYTES:
        return FrameRead(FRAME_INCOMPLETE, frame_start)
    body_length, checksum = struct.unpack(FRAME_HEADER_FORMAT, header_bytes)
    frame_end = frame_start + FRAME_HEADER_BYTES + body_length
    # Checked before reading the body, so a damaged length can't ask for 4 GiB of memory.
    if frame_end > log.length:
        return torn_unless_a_later_frame_follows(log, frame_start, next_sequence)
    body = read_at(log, frame_start + FRAME_HEADER_BYTES, body_length)
    if len(body) < body_length:
        return FrameRead(FRAME_INCOMPLETE, frame_start)
    checksum_matches = zlib.crc32(body) == checksum
    record = decode_frame_body(body) if checksum_matches else None
    if record is None:
        return classify_bad_frame(log, frame_start, frame_end, checksum_matches, next_sequence)
    return FrameRead(FRAME_RECORD, frame_end, record)


def decode_frame_body(body: bytes) -> EventRecord | None:
    """The record in a body whose checksum matched, or None when it isn't a version-1 EventRecord."""
    record = decode_event_record(body)
    if record is None or record.format_version != EVENT_RECORD_FORMAT_VERSION:
        return None
    return record


def classify_bad_frame(
    log: LogFile, frame_start: int, frame_end: int, checksum_matches: bool, next_sequence: int
) -> FrameRead:
    """A frame that failed its checksum, or whose checksum matched but whose body isn't a version-1 record: the torn
    tail, or corruption (see the top of this file)."""
    if rest_of_file_is_zero(log, frame_start):
        return FrameRead(FRAME_INCOMPLETE, frame_start)
    # A matching checksum means these are the bytes the writer wrote, so a crash didn't make them.
    if not checksum_matches and rest_of_file_is_zero(log, frame_end):
        return torn_unless_a_later_frame_follows(log, frame_start, next_sequence)
    return FrameRead(FRAME_CORRUPT, frame_start)


def torn_unless_a_later_frame_follows(log: LogFile, frame_start: int, next_sequence: int) -> FrameRead:
    """A bad frame with nothing but its own bytes and zeros left: the torn tail, unless a whole later frame follows
    its header, which proves its length is damaged."""
    if whole_later_frame_follows(log, frame_start + FRAME_HEADER_BYTES, next_sequence):
        return FrameRead(FRAME_CORRUPT, frame_start)
    return FrameRead(FRAME_INCOMPLETE, frame_start)


def whole_later_frame_follows(log: LogFile, search_start: int, next_sequence: int) -> bool:
    bytes_left = max(log.length - search_start, 0)
    window = read_at(log, search_start, min(bytes_left, LATER_FRAME_SEARCH_BYTES))
    if body_starts_as_record(window, next_sequence):
        return later_frame_at_a_field_end(window, next_sequence)
    return later_frame_at_any_offset(window, next_sequence)


def later_frame_at_a_field_end(window: bytes, next_sequence: int) -> bool:
    """The search when `window` starts as the bad frame's own record: a later frame can only start where one of its
    top-level fields ends, because anything inside a field is the record's own content."""
    crc_budget_left = LATER_FRAME_CRC_BUDGET_BYTES
    field_end = 0
    while True:
        next_field_end = field_end_at(window, field_end)
        if next_field_end is None:
            return False
        field_end = next_field_end
        outcome, crc_budget_left = try_candidate(window, field_end, next_sequence, crc_budget_left)
        if outcome == CANDIDATE_LATER_FRAME:
            return True
        if outcome == CANDIDATE_BUDGET_SPENT:
            return False


def later_frame_at_any_offset(window: bytes, next_sequence: int) -> bool:
    """The search when the bad frame's body is damaged too: every byte offset is tried, because the damaged length
    says nothing about where the next frame really starts."""
    crc_budget_left = LATER_FRAME_CRC_BUDGET_BYTES
    for candidate_start in range(len(window)):
        outcome, crc_budget_left = try_candidate(window, candidate_start, next_sequence, crc_budget_left)
        if outcome == CANDIDATE_LATER_FRAME:
            return True
        if outcome == CANDIDATE_BUDGET_SPENT:
            return False
    return False


def try_candidate(window: bytes, start: int, next_sequence: int, crc_budget_left: int) -> tuple[str, int]:
    """Whether a whole later frame starts at `start`, and the budget left after paying for its CRC-32. A candidate
    whose declared body doesn't fit in the window costs nothing."""
    bytes_left = len(window) - start
    if bytes_left < FRAME_HEADER_BYTES:
        return CANDIDATE_NOTHING, crc_budget_left
    body_length, checksum = struct.unpack_from(FRAME_HEADER_FORMAT, window, start)
    if FRAME_HEADER_BYTES + body_length > bytes_left:
        return CANDIDATE_NOTHING, crc_budget_left
    if body_length > crc_budget_left:
        return CANDIDATE_BUDGET_SPENT, crc_budget_left
    crc_budget_left -= body_length
    body_start = start + FRAME_HEADER_BYTES
    body = window[body_start : body_start + body_length]
    if zlib.crc32(body) != checksum:
        return CANDIDATE_NOTHING, crc_budget_left
    # Only a later sequence shows the writer got past the bad frame; a copy of a frame already read shows nothing.
    record = decode_frame_body(body)
    if record is not None and record.sequence >= next_sequence:
        return CANDIDATE_LATER_FRAME, crc_budget_left
    return CANDIDATE_NOTHING, crc_budget_left


def rest_of_file_is_zero(log: LogFile, start: int) -> bool:
    position = start
    while position < log.length:
        chunk = read_at(log, position, min(ZERO_SCAN_CHUNK_BYTES, log.length - position))
        if not chunk:
            return True
        if chunk.count(0) != len(chunk):
            return False
        position += len(chunk)
    return True


# ---- The protobuf field walk (frame.rs's body_starts_as_record and field_end_at) ----


@dataclass(frozen=True)
class WireField:
    """One protobuf field as the walk sees it: `value` is the number of a varint field, or the length of a
    length-delimited one."""

    number: int
    wire_type: int
    value: int
    end: int


def body_starts_as_record(data: bytes, sequence: int) -> bool:
    """True when `data` begins the way the body of record `sequence` does: format_version 1, then that sequence.
    The rest of the body may be missing (a torn frame)."""
    version = read_wire_field(data, 0)
    if version is None:
        return False
    sequence_field = read_wire_field(data, version.end)
    if sequence_field is None:
        return False
    version_matches = (
        version.number == FORMAT_VERSION_FIELD
        and version.wire_type == WIRE_TYPE_VARINT
        and version.value == EVENT_RECORD_FORMAT_VERSION
    )
    sequence_matches = (
        sequence_field.number == SEQUENCE_FIELD
        and sequence_field.wire_type == WIRE_TYPE_VARINT
        and sequence_field.value == sequence
    )
    return version_matches and sequence_matches


def field_end_at(data: bytes, start: int) -> int | None:
    """Where the whole top-level field that starts at `start` ends, or None when the bytes there aren't a field or
    it runs past the end of `data`."""
    field = read_wire_field(data, start)
    if field is None:
        return None
    return field.end


def read_wire_field(data: bytes, start: int) -> WireField | None:
    key = read_varint(data, start)
    if key is None:
        return None
    key_value, after_key = key
    number = key_value >> WIRE_TYPE_BITS
    wire_type = key_value & ((1 << WIRE_TYPE_BITS) - 1)
    if number == 0:
        return None
    if wire_type == WIRE_TYPE_VARINT:
        varint = read_varint(data, after_key)
        if varint is None:
            return None
        value, end = varint
    elif wire_type == WIRE_TYPE_FIXED64:
        value, end = 0, after_key + FIXED64_BYTES
    elif wire_type == WIRE_TYPE_FIXED32:
        value, end = 0, after_key + FIXED32_BYTES
    elif wire_type == WIRE_TYPE_LENGTH_DELIMITED:
        length = read_varint(data, after_key)
        if length is None:
            return None
        value, after_length = length
        end = after_length + value
    else:
        return None
    if end > len(data):
        return None
    return WireField(number, wire_type, value, end)


def read_varint(data: bytes, start: int) -> tuple[int, int] | None:
    """The varint at `start` and where it ends; None when the bytes end first or it is longer than 10 bytes."""
    value = 0
    for index in range(MAX_VARINT_BYTES):
        position = start + index
        if position >= len(data):
            return None
        byte = data[position]
        value |= (byte & 0x7F) << (7 * index)
        if byte & 0x80 == 0:
            return value & UINT64_MASK, position + 1
    return None


# ---- File helpers and errors ----


def read_at(log: LogFile, offset: int, size: int) -> bytes:
    """Up to `size` bytes from `offset`; fewer only when the file ends first (pread may stop early, so it loops)."""
    parts = []
    bytes_read = 0
    while bytes_read < size:
        try:
            chunk = os.pread(log.file_descriptor, size - bytes_read, offset + bytes_read)
        except OSError as error:
            raise read_error(log.path, error) from error
        if not chunk:
            break
        parts.append(chunk)
        bytes_read += len(chunk)
    return b"".join(parts)


def write_all_at(file_descriptor: int, data: bytes, offset: int) -> None:
    """Writes every byte of `data` at `offset` (pwrite may write fewer bytes than asked, so it loops)."""
    view = memoryview(data)
    while view:
        written = os.pwrite(file_descriptor, view, offset)
        view = view[written:]
        offset += written


def file_size(file_descriptor: int, path: Path) -> int:
    try:
        return os.fstat(file_descriptor).st_size
    except OSError as error:
        raise read_error(path, error) from error


def corrupt_log_error(path: Path, offset: int) -> WorkerError:
    return WorkerError(RRS_S2002, f"the event log {path} has a bad frame at byte {offset}, before its end")


def read_error(path: Path, error: OSError) -> WorkerError:
    return WorkerError(RRS_S0006, f"the event log {path} can't be opened or read: {error.strerror or error}")


def write_error(path: Path, error: OSError) -> WorkerError:
    """A full disk gets its own code; every other failed write keeps the operating system's reason."""
    if error.errno == errno.ENOSPC:
        return WorkerError(RRS_S3002, f"the disk is full while writing {path}")
    return WorkerError(RRS_S3005, f"writing {path} failed: {error.strerror or error}")
