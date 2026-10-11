# ADR-0012: Build in-house by default; external code only in five categories, always wrapped

**Status:** Accepted, 2026-10-10

## Context

- Depth is the product (§2, §3). "An in-house engine where every layer can be observed, because we wrote every layer" is a gap nobody else fills.
- Wrapped stacks bring upstream churn, license traps (unsloth-zoo is LGPL-3.0) and supply-chain risk. LiteLLM was compromised on PyPI in March 2026.
- Spike 07 showed a dependency can silently break a hard rule: Java's DJL tokenizer probed the AWS metadata address on startup to send usage telemetry, against §24's "no telemetry".
- The cost is real: in-house adds months to the 9–12 month estimate for the wrapped stack (§27). AI coding agents plus parity tests make it tractable.

## Decision

1. **External code is allowed only in five categories:**
   1. foundations that are impractical to rebuild (language runtimes, PyTorch and its GPU backends, the browser engine, the OS, the container engine);
   2. security-critical code (crypto, TLS/QUIC, SSH, database encryption, sandbox runtimes);
   3. byte-exact compatibility surfaces (tokenizers, Jinja chat templates, protobuf);
   4. external engines, as whole processes behind the adapter interface (`llama-server`, vLLM);
   5. dev tooling that never ships.
2. **Every external dependency is listed in §27** with its category, reason and replacement path. A PR that adds one updates §27 in the same PR (PR checklist).
3. **Each external library is imported in exactly one wrapper module** of the component that owns it. For example, `rastai-gateway/src/tokenizer.rs` is the only file that imports `tokenizers`. Replacing a library means changing one file.
4. **CI enforces the list:**
   - `deny.toml` (cargo-deny: crate allowlist, license allowlist, advisories);
   - `deps/allowlist-python.json` and `deps/allowlist-npm.json`;
   - `tools/checks/check-deps`;
   - hash-pinned lockfiles (`Cargo.lock`, every `uv.lock`, `pnpm-lock.yaml`);
   - external engines pinned with SHA-256 in `deps/engines/`.
5. **The allowlists name direct dependencies.** Transitive dependencies of an allowed package are accepted, but they must pass the license allowlist and the advisory check. A transitive dependency that fails gets its parent replaced or an explicit, reasoned exception in `deny.toml`.
6. **No dependency may phone home.** One that sends telemetry is configured off inside its wrapper module, with a test, or rejected.
7. **Thin utility libraries are written in-house** when that's cheap: argument parsing, the CRC table, ids, retries, the JSON-lines logger.
8. **Dropped (prior art, not dependencies):** Unsloth, TRL, PEFT, Accelerate, nnsight, TransformerLens, SAELens, circuit-tracer, rLLM, Harbor (its task format is imported), React Flow, three.js, deck.gl, uPlot, miette, Extism, Trackio (its logs are imported). **Never:** LiteLLM.
9. **Pending additions.** contracts.md §14 item 1 lists dependencies the design needs that §27 doesn't name yet: `hyper`, `hyper-util`, `tokio-rustls`, `webpki-roots`, `pyo3`, `wasm-bindgen`, `protobuf-es`, Python `protobuf`, `keyring`, `pyarrow`, and dev tooling `pytest`, `vitest`, `typescript`, `vite`, `pnpm`. Code uses these defaults until the owner adds them to §27 or picks otherwise. The spike's `reqwest`, `futures-util` and `memmap2` are not carried over.

*Note since acceptance (2026-10-10):*
- *Decision 9 is settled. The owner approved the pending additions (spec v5.4 §27, owner decision 1), each behind one wrapper file listed in contracts §2.5: `hyper`, `hyper-util`, `tokio-rustls`, `webpki-roots`, `tower`, `pyo3`, `wasm-bindgen`, `wasm-bindgen-cli`, `protobuf-es`, Python `protobuf`, `keyring`, `pyarrow`, `onnx` and `onnxscript` (under `torch.onnx`), `libc` (ADR-0033), the OS tools `setsid`, `kill`, `unshare`, `df`, `nvidia-smi` and `rocm-smi`, and dev tooling `pytest`, `vitest`, `vite`, `pnpm`, `ruff` and `prettier`. The `typescript` package is dev tooling only, as the `checkJs` checker; nobody writes TypeScript (ADR-0036). Later decisions added `miniz_oxide` and `torchvision.io.decode_image` (ADR-0037), and `mysql_async` and PyMySQL (ADR-0035). The contracts §14 item the decision cites is closed.*
- *Category 4's example: a managed vLLM engine comes after the prototype (owner decision 8). An external vLLM server is reached now through the OpenAI-compatible adapter, with `proxy` replays.*
- *The decisions are unchanged.*

**Checked by:** CI fails a PR that adds a crate, Python package or npm package not on its allowlist; a test per wrapper module that the library is imported nowhere else (`check-imports`); the offline CI test (Phase 2) that counts zero outbound connections.

## Consequences

**Good**
- Every layer is ours to observe, fix and explain. No upstream breaks us on a minor release.
- Supply-chain exposure is a short, reviewed list.
- Swapping any external piece is a one-module change.

**Costs**
- Months of extra work: model families, trainer, interpretability, RL engine and canvas are each multi-week efforts.
- We own all the bugs, including ones a mature library already fixed.
- Contributors can't reach for a familiar library. They must write it or argue for an allowlist entry.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Wrap the ecosystem (the v4.1 plan) | No uniform observation points, and constant upstream churn. The depth is the same as anyone else wrapping the same libraries. |
| Decide case by case with no rule | Drifts toward "just add the library". There's no stable answer for agents to follow. |
| Vendor external code into the repo | Still not our code. We'd own the merge burden without the understanding. |

## Links

- Spec: [§2](../spec/rastai_spec_v5.md#2-principles), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new), [§28](../spec/rastai_spec_v5.md#28-risk-register) risk 12
- Contracts: [§2 component inventory](../architecture/contracts.md#2-component-inventory) (external deps per component), [§2.5 external libraries and their wrapper files](../architecture/contracts.md#25-external-libraries-and-their-one-wrapper-file)
- Spike: [07 backend bake-off](../../spike/results/07_backend_bakeoff.md) (DJL telemetry)
- Related: [ADR-0015](0015-plugins-and-licensing.md), [ADR-0019](0019-in-house-webgl2-canvas.md), [ADR-0023](0023-owner-code-style.md), [ADR-0033](0033-unsafe-statvfs.md), [ADR-0035](0035-mysql-long-term-archive.md), [ADR-0036](0036-frontend-plain-javascript.md), [ADR-0037](0037-borrowed-decoders-and-parser-fuzzing.md)
