# ADR-0029: An in-house safetensors reader and writer with one strict contract in Rust and Python

**Status:** Accepted, 2026-10-10. Decision 1 superseded in part by [ADR-0037](0037-borrowed-decoders-and-parser-fuzzing.md), 2026-10-10.

**Superseded in part:** decision 1's "two implementations". The header contract (decision 2) has one implementation, in `rastai-formats`. Python calls `rastai_formats.check_safetensors_header` through the wheel, and `rastai_models/io/safetensors_reader.py` memory-maps the file and reads tensor bytes only after the header was accepted (ADR-0037 decision 3, contracts §2.4 rule 10). The Python writer stays. Decisions 2–5 stand; the corpus now checks one implementation from two languages.

*Note since acceptance (2026-10-10): the Rust reader lives at `crates/rastai-formats/src/safetensors/reader.rs` (control-plane layout §4.1), not `src/safetensors.rs`. The decision is unchanged.*

## Context

- Every weight file Rastai reads, from the HF cache, from imports and from shared checkpoints, is safetensors. Malformed or hostile headers are untrusted input (§23).
- The format is small: an 8-byte little-endian header length, a JSON header, and a raw data section. That makes it cheap to own under the build-in-house policy (ADR-0012), and §15 already lists the reader and writer as in-house.
- Two languages read it: the daemon (headers, model hashes, export streaming) and the Python runtime (loading tensors).
- The spike wrote a 33-file corpus (6 valid, 27 malformed). **Writing it found four gaps in the spike's own first reader.** All four bake-off backends then got 33/33 (spike 07). The Rust header parse took 0.4 ms.

## Decision

1. **Two implementations of one contract:**
   - Rust: `crates/rastai-formats/src/safetensors.rs`, ported from `spike/bakeoff/rust/src/safetensors.rs`. It uses plain file reads; the daemon only reads headers and streams tensors for export.
   - Python: `python/rastai_models/io/`, ported from the spike's `safetensors_reader.py` and `safetensors_writer.py`. It memory-maps the file and builds tensors with `torch.frombuffer`. *(Superseded in part: Python no longer checks the header itself; it calls the wheel, ADR-0037.)*
2. **The contract.** A file is rejected, with a code in `RRS-E6000–6499`, unless:
   - the header length is at most `MAX_HEADER_BYTES` (100 MiB, the reference implementation's cap) and fits inside the file, checked before anything is allocated;
   - the header is a JSON object with **no duplicate keys** and no non-finite constants (`NaN`, `Infinity`);
   - every dtype is in the known list, and every shape entry and offset is a **real integer** (booleans and floats are rejected);
   - each tensor's byte size, computed as product(shape) × dtype size **with overflow checks**, equals `end − begin` (a shape of [2³², 2³²] must not wrap to 0);
   - the tensors' `[begin, end)` ranges **tile the data section exactly**, with no overlaps, gaps or trailing bytes;
   - `__metadata__`, if present, maps strings to strings only.
3. **The writer** emits sorted tensor names, contiguous offsets, and a header padded with spaces to an 8-byte boundary. It writes to a temporary name and renames when done.
4. **Provenance** goes in `__metadata__`: a pointer back to the recipe on every export (§13).
5. **One corpus, both languages:** `fixtures/safetensors/` holds the 33 spike files, each with its expected code. Rust and Python must give the same verdict and code on every file.

**Checked by:** the corpus test in both languages; a cargo-fuzz target on the Rust header parser (`crates/rastai-formats/fuzz/`); round-trip property tests (write → read → identical tensors and metadata); the checksum of the 2 GB Gemma file matches between the two readers.

## Consequences

**Good**
- Hostile or broken weight files fail early, with a precise code and path, before allocation.
- Rust and Python agree by construction, so the daemon's model hash and the worker's tensors refer to the same bytes.
- No external dependency for a format that is a few hundred lines.

**Costs**
- Two implementations to keep in step. The shared corpus is the guard, and every new edge case is added to it as a file.
- Being stricter than other tools means a few files other loaders accept will be refused (trailing bytes, for example). The diagnostic says exactly why.

## Alternatives considered

| Alternative | Why not |
|---|---|
| The HF `safetensors` crate and Python package | An external dependency outside the five categories, for a format we can own in a few hundred lines. Its errors wouldn't map to our codes in both languages. |
| One Rust implementation used from Python through the PyO3 wheel | Python needs zero-copy tensors from memory-mapped files on the tensor path. The header rules are shared through the corpus instead. |
| A lenient reader | Header bugs become memory bugs or silent corruption. The spike found four gaps in a reader that "worked". |

## Links

- Spec: [§15](../spec/rastai_spec_v5.md#15-file-formats), [§23](../spec/rastai_spec_v5.md#23-validation) (strict safetensors contract), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§1 port map](../architecture/contracts.md#1-repository-tree), [§10 RRS-E ranges](../architecture/contracts.md#10-error-catalog), [§12.5 fixtures](../architecture/contracts.md#12-naming-and-code-conventions)
- Spike: [07 backend bake-off](../../spike/results/07_backend_bakeoff.md)
- Related: [ADR-0012](0012-build-in-house-policy.md), [ADR-0024](0024-no-pickle-checkpoints.md), [ADR-0032](0032-document-files-and-validation.md), [ADR-0037](0037-borrowed-decoders-and-parser-fuzzing.md)
