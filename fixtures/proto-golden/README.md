# Golden protobuf messages

One filled instance of every message in `proto/rastai/`, one folder per package (contracts §12.5; layout `rust-node-gateway-proto.md` §3.5):

- `<package>/<MessageName>.json`: the message in the protobuf JSON mapping, with proto field names (`snake_case`), 64-bit integers as strings and enums by name, the form contracts §4 stores wire types in documents. This is the file people write.
- `<package>/<MessageName>.binpb`: the same message in the binary wire format, written from the `.json` by `buf convert`.

Every field is set to a value other than its default, so a field missing from one language's generated code shows up as a difference. An `optional` field set to its default (`ResourceRequest.may_reclaim_evictable = false`) is still on the wire, which is the point of `optional`.

The tests that read them:

| Language | Test | What it checks |
|---|---|---|
| Rust | `crates/rastai-proto/tests/golden.rs` | decode and encode give the `.binpb` bytes back; prost drops fields it doesn't know, so a stale struct fails |
| Python | `python/rastai_proto/tests/test_golden.py` | decode, drop unknown fields, encode: the same bytes; every message of the package has a golden |
| JavaScript | `packages/proto/src/golden.test.js` | the same bytes without unknown fields; the `.json` encodes to the `.binpb` and the `.binpb` prints as the `.json`; every field is set; every message has a golden |

All three compare against the same `.binpb` files, so their encoders agree byte for byte.

## Adding or changing a golden

1. Write or edit `<package>/<MessageName>.json`.
2. From the repository root, write its binary form:

   ```
   buf convert . --type rastai.<package>.v1.<MessageName> --from fixtures/proto-golden/<package>/<MessageName>.json --to fixtures/proto-golden/<package>/<MessageName>.binpb
   ```

3. Run `just test-rust test-python test-js`. Rust lists its messages by hand (prost has no reflection), so a new message also gets a line in `golden.rs`.

Field numbers are frozen, and an old `.binpb` keeps decoding after a field is added (layout 3.3): add a new golden beside the old one rather than rewriting it when a message read from storage changes.
