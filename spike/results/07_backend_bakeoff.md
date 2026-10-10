# Experiment 07: Rust vs Java for the backend

Same command-line contract, idiomatic code on each side. Rust: tokio, axum, reqwest, serde_json, rusqlite, HF `tokenizers`. Java 25: JDK HttpServer on virtual threads, java.net.http, Jackson, sqlite-jdbc, DJL tokenizers (JNI to the same Rust library).

## Correctness

| Contestant | Safetensors verdicts | Crashes | Gemma 2 GB checksum | Tokenizer mismatches (162 cases) | End-to-end token IDs identical |
|---|---|---|---|---|---|
| Rust | 33/33 | 0 | 3991664908 | 0 | True |
| Java, JVM defaults | 33/33 | 0 | 3991664908 | 0 | True |
| Java, JVM desktop-tuned | 33/33 | 0 | 3991664908 | 0 | True |
| Java, GraalVM native image | 33/33 | 0 | 3991664908 | 0 | True |

## Speed of the building blocks

| Contestant | Header parse + validation | 2 GB checksum pass | Tokenizer load | Tokens/s (1 thread) |
|---|---|---|---|---|
| Rust | 0.4 ms | 48 ms | 556 ms | 2.83 M |
| Java, JVM defaults | 31.3 ms | 51 ms | 931 ms | 1.30 M |
| Java, JVM desktop-tuned | 31.7 ms | 51 ms | 948 ms | 1.26 M |
| Java, GraalVM native image | 1.0 ms | 728 ms | 1038 ms | 1.26 M |

## Gateway under load (fake instant upstream, 64 tokens per streamed reply, 5 s warm-up, 10 s per level)

| Contestant | Startup | Idle memory | c=1 req/s (p99 ms) | c=16 req/s (p99 ms) | c=64 req/s (p99 ms) | c=256 req/s (p99 ms) | CPU µs/request (c=64) | Peak memory |
|---|---|---|---|---|---|---|---|---|
| Rust | 533 ms | 252 MiB | 2,923 (0.6) | 16,761 (4.0) | 17,868 (8.1) | 17,724 (28.1) | 781 | 360 MiB |
| Java, JVM defaults | 1163 ms | 705 MiB | 1,786 (0.8) | 12,166 (4.4) | 14,376 (9.3) | 14,183 (26.3) | 983 | 1901 MiB |
| Java, JVM desktop-tuned | 1181 ms | 632 MiB | 1,810 (0.9) | 11,777 (4.5) | 13,254 (10.1) | 10,767 (55.0) | 995 | 814 MiB |
| Java, GraalVM native image | 1058 ms | 435 MiB | 1,472 (1.1) | 10,678 (4.5) | 12,373 (11.0) | 10,549 (37.4) | 1094 | 570 MiB |

## Building and shipping

| Contestant | Clean build | Incremental release build | Incremental dev build | Shippable | Size |
|---|---|---|---|---|---|
| Rust | 32 s | 2.0 s | 0.4 s | one binary | 9 MiB |
| Java (JVM) | 4 s | 4.2 s | 1.2 s | jar + jlink runtime | 98 MiB |
| Java, GraalVM native image | 28 s | — | — | one binary | 37 MiB |

Lines of code (non-blank, same features): Rust 550, Java 621.

## End to end through the real model

Answer via every gateway: "The sky appears blue due to a phenomenon called Rayleigh scattering, where shorter wavelengths of light (like blue and violet) are scattered more by the atmosphere's molecules than longer wavelengths.  This scattered blue light reaches our eyes from all directions,"

## Environment

```
recorded_at: 2026-10-10T10:00:57+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
