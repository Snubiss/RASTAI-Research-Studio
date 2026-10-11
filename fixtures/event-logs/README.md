# Event-log fixtures

Small `events.log` files in the `RRSEVLG1` format (contracts §7.1): the 8-byte header, then frames of `body_length: u32 LE` | `checksum: u32 LE` (CRC-32/IEEE of the body, Python's `zlib.crc32`) | `body` (one protobuf `rastai.events.v1.EventRecord`).

The first four files hold the same short training run: `run.state_changed` (running), `run.metrics` at steps 1 and 2 (`train.loss`), and `checkpoint.saved` at step 2, for run `run_01JA2B3C4D5E6F7G8H9J0K1M2S` and job `job_01JA2B3C4D5E6F7G8H9J0K1M2Q`.

| File | What is in it | A reader reports |
|---|---|---|
| `good.log` | the four records, sequences 1 to 4 | four records, then the end of the file |
| `torn_tail.log` | the same, with the fourth frame cut halfway through its body (the writer died mid-write) | three records, then `Incomplete` at the end of the third frame; a writer that opens it for append truncates there first (`RRS-S2001`, info) |
| `corrupt_middle.log` | the four frames, with one bit flipped in the second frame's body | one record, then `Corrupt` at the start of the second frame (`RRS-S2002`) |
| `sequence_gap.log` | records 1, 2 and 4, every frame whole | two records, then a sequence gap: expected 3, found 4 (`RRS-S2003`) |
| `from_python.log` | written by the Python worker's `EventLogWriter`: seven records of every durable event type a worker writes (run states, metrics, a checkpoint, an artifact with a blob, a diagnostic), including a metric of 0.0 and metric names where one starts the other, the two map cases where Python's own protobuf serializer and prost disagree | seven records, then the end of the file; each frame is the one the Rust writer writes for that record |

## Who reads them

| Language | Test |
|---|---|
| Rust | `crates/rastai-store/tests/event_log_fixtures.rs`: the reader's verdict for each file, and the writer's append after the torn tail |
| Python | `python/rastai_worker/tests/test_event_log.py`: the writer writes `good.log` byte for byte, cuts `torn_tail.log`'s tail, refuses `corrupt_middle.log` (`RRS-S2002`) and `sequence_gap.log` (`RRS-S2003`), and writes `from_python.log` from its recipe |
| Rust | `crates/rastai-store/tests/python_log_compat.rs`: `from_python.log` reads frame for frame, each frame is prost's, and the Rust writer writes the same file |

## Changing them

The first four files come from the recipes in `crates/rastai-store/tests/event_log_fixtures.rs`; its test `committed_fixtures_match_their_recipes` fails when a file and its recipe differ. After a deliberate change to the format or a recipe, rewrite the files with:

```
cargo test -p rastai-store --test event_log_fixtures -- --ignored write_event_log_fixtures
```

`from_python.log` comes from the recipe in `python/rastai_worker/tests/test_event_log.py`, whose test fails when the file and the recipe differ; `python_log_compat.rs` holds the same records in Rust, so the two recipes change together. Rewrite it from the repository root with:

```
.dev/envs/cpu/bin/python python/rastai_worker/tests/test_event_log.py
```

The format itself never changes in place: a new record layout is a new `format_version`, and old files keep reading.
