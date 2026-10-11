# Event-log fixtures

Small `events.log` files in the `RRSEVLG1` format (contracts §7.1): the 8-byte header, then frames of `body_length: u32 LE` | `checksum: u32 LE` (CRC-32/IEEE of the body, Python's `zlib.crc32`) | `body` (one protobuf `rastai.events.v1.EventRecord`).

Every file holds the same short training run: `run.state_changed` (running), `run.metrics` at steps 1 and 2 (`train.loss`), and `checkpoint.saved` at step 2, for run `run_01JA2B3C4D5E6F7G8H9J0K1M2S` and job `job_01JA2B3C4D5E6F7G8H9J0K1M2Q`.

| File | What is in it | A reader reports |
|---|---|---|
| `good.log` | the four records, sequences 1 to 4 | four records, then the end of the file |
| `torn_tail.log` | the same, with the fourth frame cut halfway through its body (the writer died mid-write) | three records, then `Incomplete` at the end of the third frame; a writer that opens it for append truncates there first (`RRS-S2001`, info) |
| `corrupt_middle.log` | the four frames, with one bit flipped in the second frame's body | one record, then `Corrupt` at the start of the second frame (`RRS-S2002`) |
| `sequence_gap.log` | records 1, 2 and 4, every frame whole | two records, then a sequence gap: expected 3, found 4 (`RRS-S2003`) |

## Who reads them

| Language | Test |
|---|---|
| Rust | `crates/rastai-store/tests/event_log_fixtures.rs`: the reader's verdict for each file, and the writer's append after the torn tail |
| Python | the `rastai_worker` event-log writer's tests (M1-39): the same files, frame for frame |

## Changing them

The bytes come from the recipes in `crates/rastai-store/tests/event_log_fixtures.rs`; its test `committed_fixtures_match_their_recipes` fails when a file and its recipe differ. After a deliberate change to the format or a recipe, rewrite the files with:

```
cargo test -p rastai-store --test event_log_fixtures -- --ignored write_event_log_fixtures
```

The format itself never changes in place: a new record layout is a new `format_version`, and old files keep reading.
