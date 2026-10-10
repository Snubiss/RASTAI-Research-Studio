# ADR-0016: The backend stays Rust after the Java bake-off

**Status:** Accepted, 2026-10-10

## Context

- The spec chose Rust for the daemon, node, gateway, store and CLI. The owner asked for Java to be tested too, before code is written.
- The choice doesn't affect compute: models, lenses and training run in the Python workers either way.
- Spike 07 built the backend's own work in both languages, written the usual way for each, with the same command-line contract (`spike/bakeoff/rust`, `spike/bakeoff/java`):
  - the strict safetensors reader;
  - gateway tokenization;
  - a streaming OpenAI-compatible gateway that tokenizes, forwards token IDs to `llama-server`, relays chunks and logs token IDs to SQLite.
  Java ran three ways: the JVM with defaults, the JVM tuned like a desktop app, and a GraalVM native image.
- **Correctness was a tie.** All four were 33/33 on the safetensors corpus with no crashes, produced the same checksum on the 2 GB model and identical token IDs on the 162-case corpus, and had no errors in 1.7 million load-test requests.
- **Speed and footprint:**

  | | Rust | JVM defaults | JVM tuned | GraalVM native |
  |---|---|---|---|---|
  | Gateway req/s at 1 / 64 clients | 2,923 / 17,868 | 1,786 / 14,376 | 1,810 / 13,254 | 1,472 / 12,373 |
  | p99 at 256 clients | 28 ms | 26 ms | 55 ms | 37 ms |
  | CPU per request | 781 µs | 983 µs | 995 µs | 1,094 µs |
  | Idle / peak memory | 252 / 360 MiB | 705 / 1,901 MiB | 632 / 814 MiB | 435 / 570 MiB |
  | Startup to first healthy response | 0.5 s | 1.2 s | 1.2 s | 1.1 s |
  | Tokenizer speed | 2.83 M tok/s | 1.30 M | 1.26 M | 1.26 M |
  | Shippable | 9 MiB binary | 98 MiB | 98 MiB | 37 MiB binary |
  | Clean / incremental dev build | 32 s / 0.4 s | 4 s / 1.2 s | 4 s / 1.2 s | 28 s / — |

- **Java pitfalls found:**
  - DJL, Java's practical tokenizer, silently truncates long inputs by default (7 of 162 cases);
  - DJL phones home (it probes 169.254.169.254 for AWS telemetry, costing ~4 s of timeouts off AWS);
  - DJL wraps the same Rust `tokenizers` library through JNI, in an older version;
  - GraalVM runs class initializers at build time, so a runtime socket setting was silently ignored, causing 40 ms stalls and a hang under load.

## Decision

1. **The backend is Rust:** `rastai-cli`, `rastai-server`, `rastai-node`, `rastai-gateway`, `rastai-store`, `rastai-transport`, `rastai-sandbox`, `rastai-mcp`, `rastai-formats`, `rastai-core`, `rastai-diag`, on tokio and axum.
2. **No JVM in the product.**
3. **Lessons carried into the Rust build:**
   - streaming HTTP sockets set `TCP_NODELAY` (both languages stalled 40 ms per reply without it);
   - SQLite writes go through one writer thread per database (the first Rust version convoyed on a mutex at 256 clients; ADR-0003);
   - library choice can swamp language choice (an AVX-512 CRC crate made Rust's 2 GB checksum 4.6× faster). Hot paths are measured before choosing an implementation. BLAKE3 covers whole-file hashing; the in-house CRC table covers small event-log frames, where speed doesn't matter.
4. **Re-opening this decision needs a new ADR** that either beats these numbers or shows the backend's job has changed.

## Consequences

**Good**
- One 9 MiB static binary to ship and push to nodes. Low idle memory next to GPU workloads.
- The most exact-sensitive job, tokenization, runs in its native library with no JNI layer.
- Faster day-to-day builds (0.4 s incremental dev build vs 1.2 s).

**Costs**
- Clean builds are slow (32 s vs 4 s). CI caches build artifacts.
- Async Rust has sharp edges. The code-style rules (enums with `match` instead of async trait objects, ADR-0023) keep it plain.
- Fewer contributors know Rust than Java. The code is about the same size (550 vs 621 lines in the bake-off).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Java on the JVM | 2–5× the memory, 98 MiB to ship, slower tokenization, and the DJL pitfalls above. |
| Java as a GraalVM native image | Smaller, but slower than the JVM under load, with build-time initialization surprises. |
| Python backend | The GIL, couples crashes with compute, and makes a large shippable. |
| Go | Not tested. Its tokenizer story is the same Rust library through cgo. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 14, [§16](../spec/rastai_spec_v5.md#16-architecture-overview), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Spike: [README section 07](../../spike/README.md), [07 backend bake-off results](../../spike/results/07_backend_bakeoff.md)
- Related: [ADR-0001](0001-process-layout.md), [ADR-0003](0003-storage-sqlite-vault-blobs.md), [ADR-0008](0008-gateway-owns-tokenization.md), [ADR-0023](0023-owner-code-style.md)
