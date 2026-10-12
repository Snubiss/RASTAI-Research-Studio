# Golden protobuf messages

One filled instance of every message in `proto/rastai/`, one folder per package (contracts §12.5; layout `rust-node-gateway-proto.md` §3.5):

- `<package>/<MessageName>.json`: the message in the protobuf JSON mapping, with proto field names (`snake_case`), 64-bit integers as strings and enums by name, the form contracts §4 stores wire types in documents. This is the file people write.
- `<package>/<MessageName>.binpb`: the same message in the binary wire format, written from the `.json` by `buf convert`.

Every field is set to a value other than its default, so a field missing from one language's generated code shows up as a difference. An `optional` field set to its default (`ResourceRequest.may_reclaim_evictable = false`) is still on the wire, which is the point of `optional`. A `oneof` can hold only one of its fields, so its golden sets one (`EventPayload`, `RunStateChanged`). A message with no fields (`ListEventTypesRequest`, `HelloRequest`, ...) has `{}` and an empty `.binpb`; the Rust test lists them in `FIELDLESS_MESSAGES`. Values are realistic but illustrative: they show the shape, not real records.

Extra goldens live in subfolders and aren't counted as "one per message":

- `commands/without_stability/CommandDescriptor` and `events/without_stability/EventTypeDescriptor`: descriptors as a daemon that predates `stability` sends them. They decode as `STABILITY_UNSPECIFIED`, which readers treat as `PREVIEW` (layout 3.2 and 3.5).
- `commands/connect_json/`: the Connect JSON of the six `CommandService` messages, which `rastai-server`'s hand-written codec must equal (control-plane layout 11.3). `cases.json` has three lists: `encode` (a `.binpb`, either one of this folder's or a package golden named `../`, and the exact compact text protobuf-es's `toJsonString` writes for it with `useProtoFieldName`), `decode` (an input protobuf-es's `fromJsonString` accepts, such as connect-es's camelCase names, and the text it writes back) and `refuse` (an input protobuf-es refuses, with the code and JSON-pointer path the daemon refuses it with). `packages/proto/src/golden.test.js` checks every case against protobuf-es; `crates/rastai-server/tests/connect_protocol.rs` checks the daemon's codec against every case.

The tests that read them:

| Language | Test | What it checks |
|---|---|---|
| Rust | `crates/rastai-proto/tests/golden.rs` | decode and encode give the `.binpb` bytes back; prost drops fields it doesn't know, so a stale struct fails; the stability of the descriptor goldens |
| Python | `python/rastai_proto/tests/test_golden.py` | decode, drop unknown fields, encode: the same bytes; every message of each package has a golden; the stability of the descriptor goldens |
| JavaScript | `packages/proto/src/golden.test.js` | the same bytes without unknown fields; the `.json` encodes to the `.binpb` and the `.binpb` prints as the `.json`; every field is set (one field per oneof); every message has a golden; the stability of the descriptor goldens |
| Rust | `crates/rastai-server/tests/connect_protocol.rs` | the daemon's Connect JSON: each `commands/` golden and `common/Diagnostic` written as its `.json`, and every case of `commands/connect_json/cases.json` |

All three compare against the same `.binpb` files, so their encoders agree byte for byte.

## Adding or changing a golden

1. Write or edit `<package>/<MessageName>.json`.
2. From the repository root, write its binary form:

   ```
   buf convert . --type rastai.<package>.v1.<MessageName> --from fixtures/proto-golden/<package>/<MessageName>.json --to fixtures/proto-golden/<package>/<MessageName>.binpb
   ```

3. Run `just test-rust test-python test-js`. Rust lists its messages by hand (prost has no reflection), so a new message also gets a line in `golden.rs`. A new package gets a folder here and an entry in each test's package list.

A Connect JSON case is added to `commands/connect_json/cases.json`. An `encode` case needs its binary: write the message's JSON to a file and run step 2 with `--to fixtures/proto-golden/commands/connect_json/<MessageName>.<case>.binpb`. Then run `just test-js`: a case protobuf-es disagrees with fails there with protobuf-es's own text, before the Rust test reads it.

Field numbers are frozen, and an old `.binpb` keeps decoding after a field is added (layout 3.3): add a new golden beside the old one rather than rewriting it when a message read from storage changes.
