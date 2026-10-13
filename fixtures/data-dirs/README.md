# Data-folder fixtures

Small `<data>` folders (contracts §8.1) for the offline rebuild of `rastai.db` (`rastai storage rebuild-index`,
layout rust-control-plane.md §13.2, flows/platform.md §6.4). Each holds only what a rebuild reads: run folders, the
daemon's own event log and the blob store. There is no `rastai.db`: the tests index the folder live first, with the
writes a daemon sends, and then rebuild it.

## `two_runs/data/`

| File | What is in it |
|---|---|
| `runs/run_01JA2B3C4D5E6F7G8H9J0K1M2S/run.json` | run A's record (`rastai.run_record/1`), with the `job_spec` of job `job_01JA2B3C4D5E6F7G8H9J0K1M2Q` and `created_at` to the millisecond |
| `runs/run_01JA2B3C4D5E6F7G8H9J0K1M2S/events.log` | run A's log: running, two `run.metrics`, a best `checkpoint.saved`, an `artifact.created` (`exported_as` from the checkpoint) whose bytes are the one blob, completed |
| `runs/run_01JA2B3C4D5E6F7G8H9J0K1M2T/run.json` | run B's record; its job failed before it started, so it has no log |
| `daemon/events.log` | the daemon's records: the node's connect, run A's job states, two `command.finished`, a `pin.changed` on the checkpoint, `run.tags_changed`, run B's failed job and the run end the daemon wrote for it, and a `plugin.state_changed` that must not move `plugins.state` |
| `blobs/e9/e94b...b069` | the exported file, named by its BLAKE3 |

## Who reads it

`crates/rastai-store/tests/rebuild_index.rs`: every rebuildable table of the rebuilt index equals the live one row by
row, and a corrupt frame (`RRS-S2002`), a record a run's log may not carry (`RRS-S2004`), a missing or changed blob
(`RRS-S1002`, `RRS-S1001`), an old index that isn't a database and a `run.json` naming another run each end where
the flow says. rastai-cli's `tests/rebuild_index.rs` makes the same comparison against an index the real daemon wrote.

## Changing it

The files come from the recipe in `crates/rastai-store/tests/rebuild_index.rs`; its test
`committed_fixture_matches_its_recipe` fails when the files and the recipe differ. After a deliberate change, rewrite
them with:

```
cargo test -p rastai-store --test rebuild_index -- --ignored write_rebuild_fixture
```
