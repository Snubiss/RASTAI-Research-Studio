# ADR-0037: Borrowed decoders for DEFLATE and images; one implementation and a fuzz target for every in-house parser

**Status:** Accepted, 2026-10-10. Supersedes in part [ADR-0029](0029-strict-safetensors-contract.md) decision 1 (the Python header checks).

## Context

- Parsers of untrusted input are where memory-safety and denial-of-service bugs live (§23, §24). Every zip, image, safetensors file and imported conversation crosses one.
- The drafts planned two decoders in-house:
  - **DEFLATE** in `rastai-formats` (about 400 lines, plan M2-22) for zip archives: pinned llama-server builds, `.rrsplugin` and `.rrsreplay` files, imports;
  - **PNG and JPEG** in `rastai_data/image_decode.py` (about 5 days: chunks, filters, interlacing, baseline and progressive Huffman decoding), with Python entropy decoding at tens of milliseconds per small JPEG.
- Neither is depth the product sells. DEFLATE is a byte-exact compatibility surface (§27 category 3); image decoding sits under PyTorch's own image stack.
- `miniz_oxide` is a pure-Rust DEFLATE implementation (MIT OR Zlib OR Apache-2.0). `torchvision.io.decode_image` comes from the PyTorch project, already a foundation, and returns tensors directly.
- ADR-0029 kept two implementations of the strict safetensors header contract, one in Rust and one in Python, held in step by a shared corpus. Two implementations of one check can still disagree on a case the corpus doesn't have. The Python runtime already loads the `rastai_formats` wheel (ADR-0004).

## Decision

1. **DEFLATE comes from `miniz_oxide`**, imported only in `crates/rastai-formats/src/zip/inflate.rs`; `deny.toml` `wrappers` pins it to `rastai-formats`.
   - The zip container stays in-house (`zip/reader.rs`): the central directory, every size and ratio cap checked before anything is inflated, zip-slip paths refused (`RRS-E5001`, `RRS-E5002`).
   - `inflate.rs` stops at the entry's declared uncompressed size, so a lying header can't inflate past its cap.
2. **Images decode through `torchvision.io.decode_image`**, imported only in `python/rastai_data/image_decode.py`, behind `decode_image(encoded: bytes) -> Tensor` (uint8, `[channels, height, width]`).
   - PNG and JPEG are accepted; the wrapper checks the leading magic bytes and refuses anything else with `RRS-D1001`. A file that fails to decode is a failed row (`RRS-D1002` when more than 1% fail).
   - Limits on encoded bytes and decoded pixels are constants at the top of `image_decode.py`.
   - Decoding runs only in worker jobs (data import, the image cache build, image chat inputs), never in the daemon, so a hostile image costs at most one job.
   - torchvision is pinned in each backend lock beside the matching torch, and brings `pillow` and `numpy` in as transitive packages.
3. **One implementation per in-house parser.** When Rust and Python both need a check, `rastai-formats` owns it and Python calls the wheel (contracts §2.4 rule 10):
   - **safetensors headers:** Python calls `rastai_formats.check_safetensors_header`, then `rastai_models/io/safetensors_reader.py` memory-maps and reads tensor bytes only after the header was accepted. Both languages report the same `RRS-E60xx` code because there is one implementation;
   - GGUF byte encoding, BLAKE3 and document validation work the same way.
4. **Every remaining in-house parser of untrusted input has a fuzz target,** run nightly for 10 minutes each (contracts §12.5): `json_reader`, `read_document`, `safetensors_header`, `gguf_header`, `zip_reader`, `inflate` (the wrapper's caps), `conversation_import`, `base32_decode` and `rfc3339_parse` (rastai-core's id and timestamp parsers, added by M1-20), `event_frame`, `request_gate`, `connect_json`, `sse_reader`, `jsonrpc_decode`, `handshake`, `archived_frame`, `connect_request` and `exec_stream` (rastai-sandbox; the CONNECT line is bytes written by untrusted sandboxed code), `tool_call_parse` (rastai-gateway; model output is untrusted text). That is 19 targets. Python parsers (no fuzzing dependency) get seeded mutation tests marked `fuzz`: the lm-eval YAML subset, the run-log readers, the CSV and JSONL readers. JavaScript parsers get vitest seeded mutation tests: `parse-markdown.js` and the iframe bridge's message validation (contracts §12.5). A new parser lands with its target in the same PR.
5. **Replacement paths** (§27): an in-house inflate or image decoder can come back behind the same function if either library ever fails us. No caller changes.

**Checked by:**
- `tests/zip.rs`: stored and deflate archives read; zip slip → `RRS-E5001`; a bomb ratio → `RRS-E5002` before inflating; inflate stops at the declared size;
- `tests/test_image_decode.py`: the `fixtures/images/` corpus decodes to its expected pixels; GIF, WebP and a truncated file are refused, never half-decoded;
- the safetensors corpus (`fixtures/safetensors/`, 33 files) through Rust and through the wheel from Python gives the same verdict and code on every file;
- a Python test that `safetensors_reader.py` reads no tensor bytes from a file the wheel rejected;
- `check-imports` and `deny.toml`: `miniz_oxide` and `torchvision.io` imported nowhere else;
- `nightly.yml` runs every fuzz target; `docs/security/threat-model.md` names a target for every boundary parser (M1-56).

## Consequences

**Good**
- About 400 lines of Rust and 5 days of Python parser work are gone, with the risk that came with them.
- Image decoding is native speed, so the image cache builds in seconds, not minutes.
- One safetensors header check, so the daemon and the worker can't disagree about a file.
- Every parser we still own is fuzzed. The list is the one in contracts §12.5, and a new parser adds its row there.

**Costs**
- `pillow` and `numpy` enter every backend lock. Their licenses must pass the allowlist; `pillow`'s MIT-CMU (HPND) license may need a named entry in `deps/allowlist-python.json`.
- The image path runs native decoder code we didn't write. Running it only in jobs bounds the damage.
- Python now needs the `rastai_formats` wheel to read any safetensors file, including in the standalone SDK. It already needed the wheel for documents.
- Fuzz targets cost nightly CI time (19 targets × 10 minutes).

## Alternatives considered

| Alternative | Why not |
|---|---|
| In-house DEFLATE | 400 lines of decompressor, a classic bug source, for a byte-exact surface we can't improve. |
| The `zip` or `flate2` crate | The container and its limits must be ours; `flate2` is one more layer over `miniz_oxide`. |
| In-house PNG and JPEG decoders | About 5 days, slow Python entropy decoding, and parser risk. |
| Pillow directly | Another API for the same native code; torchvision returns tensors and sits under the PyTorch row. |
| Two safetensors header implementations (ADR-0029 as accepted) | They can drift on a case the corpus lacks. One implementation agrees by construction. |
| Fuzz only the parsers on network boundaries | Files from users and the Hub are just as untrusted. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 19, [§15](../spec/rastai_spec_v5.md#15-file-formats), [§23](../spec/rastai_spec_v5.md#23-validation), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§2.4 rule 10](../architecture/contracts.md#24-layering-rules), [§2.5 miniz_oxide and torchvision rows](../architecture/contracts.md#25-external-libraries-and-their-one-wrapper-file), [§12.5 fixtures and fuzzing](../architecture/contracts.md#125-fixtures-and-fuzzing), [§15.11 names replaced](../architecture/contracts.md#1511-names-rejected-or-replaced)
- Plan: M2-13 (safetensors), M2-22 (zip and inflate), M6-E07 (images before the ViT test), M1-56 (threat model)
- Related: [ADR-0012](0012-build-in-house-policy.md), [ADR-0029](0029-strict-safetensors-contract.md), [ADR-0032](0032-document-files-and-validation.md), [ADR-0024](0024-no-pickle-checkpoints.md)
