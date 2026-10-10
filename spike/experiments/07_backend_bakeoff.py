"""
Experiment 07: Rust vs Java for Rastai's backend (owner's question, 2026-10-10).

The spike's other experiments don't depend on the backend language: models, lenses and training run in the
Python workers either way. So this compares what the backend itself does, implemented idiomatically in both
languages with the same command-line contract (bakeoff/rust, bakeoff/java):

  1. rastai-formats core: a strict safetensors reader. Verdicts on 33 hand-built files (6 valid, 27 malformed)
     must match the Python reference exactly, never crash, and checksum the real 2 GB Gemma file identically.
  2. Gateway tokenization: token IDs on the 162-case stress corpus must match the reference, plus speed.
  3. Gateway: an OpenAI-compatible streaming endpoint that tokenizes, forwards token IDs upstream, relays
     chunks and logs token IDs to SQLite. Load-tested against an instant fake upstream (so the gateway, not
     the GPU, is measured), and checked end to end against the real model on llama-server.
  4. Footprint: build times, shippable size, startup time, memory.

Java runs three ways: the JVM with default settings, the JVM tuned like a desktop app would be
(-Xmx256m -XX:+UseSerialGC), and a GraalVM native image (a single native binary, like Rust's).
The load generator and fake upstream are shared Rust tools, identical for every contestant.

Run:  uv run python experiments/07_backend_bakeoff.py
"""

import json
import os
import shutil
import signal
import sqlite3
import statistics
import subprocess
import time
import urllib.request
from pathlib import Path

from rastai_spike.bakeoff_fixtures import FIXTURES_FOLDER, SAFETENSORS_FOLDER, write_safetensors_fixtures, write_tokenizer_fixtures
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.llama_server import LlamaServer
from rastai_spike.reporting import environment_fingerprint, write_results

EXPERIMENT_NAME = "07_backend_bakeoff"
SPIKE_FOLDER = Path(__file__).resolve().parents[1]
SCRATCH_FOLDER = SPIKE_FOLDER / ".scratch" / EXPERIMENT_NAME
VENDOR_FOLDER = SPIKE_FOLDER / "vendor"
RUST_FOLDER = SPIKE_FOLDER / "bakeoff" / "rust"
JAVA_FOLDER = SPIKE_FOLDER / "bakeoff" / "java"
RUST_BINARY = RUST_FOLDER / "target" / "release" / "bakeoff-rust"
JAVA_JAR = JAVA_FOLDER / "target" / "bakeoff-java.jar"
JAVA_NATIVE_BINARY = JAVA_FOLDER / "target" / "bakeoff-java-native"
JDK_HOME = VENDOR_FOLDER / "jdk-temurin-25.0.4"
GRAALVM_HOME = VENDOR_FOLDER / "graalvm-community-25.0.4"
MAVEN = VENDOR_FOLDER / "apache-maven-3.9.16" / "bin" / "mvn"
CARGO = Path.home() / ".cargo" / "bin" / "cargo"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
GGUF_REPOSITORY = "ggml-org/gemma-3-1b-it-GGUF"
GGUF_FILE = "gemma-3-1b-it-f16.gguf"

MOCK_UPSTREAM_PORT = 18100
GATEWAY_PORT = 18101
LLAMA_SERVER_PORT = 18089
LOOPBACK = "127.0.0.1"
TOKENIZER_REPEATS = 5
SAFETENSORS_READ_RUNS = 3
STARTUP_TRIALS = 5
HEALTH_POLL_SECONDS = 0.002
STARTUP_TIMEOUT_SECONDS = 60
WARMUP_SECONDS = 5
LOAD_SECONDS = 10
LOAD_CONCURRENCIES = [1, 16, 64, 256]
TOKENS_PER_REQUEST = 64
END_TO_END_TOKENS = 48
END_TO_END_QUESTION = "Explain in two sentences why the sky is blue."
CLOCK_TICKS_PER_SECOND = os.sysconf("SC_CLK_TCK")
MIB = 1024**2

JAVA = str(JDK_HOME / "bin" / "java")
NATIVE_ACCESS = "--enable-native-access=ALL-UNNAMED"  # DJL and sqlite-jdbc load native libraries
CONTESTANTS = {
    "Rust": [str(RUST_BINARY)],
    "Java, JVM defaults": [JAVA, NATIVE_ACCESS, "-jar", str(JAVA_JAR)],
    "Java, JVM desktop-tuned": [JAVA, NATIVE_ACCESS, "-Xmx256m", "-XX:+UseSerialGC", "-jar", str(JAVA_JAR)],
    "Java, GraalVM native image": [str(JAVA_NATIVE_BINARY)],
}


# ----------------------------------------------------------------------------------------------
# Small helpers
# ----------------------------------------------------------------------------------------------

def run(command: list[str], cwd: Path | None = None, environment: dict | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, env=environment, capture_output=True, text=True, check=check)


def timed(command: list[str], cwd: Path | None = None, environment: dict | None = None) -> float:
    start = time.perf_counter()
    run(command, cwd, environment)
    return time.perf_counter() - start


def run_json(command: list[str]) -> dict:
    return json.loads(run(command).stdout.strip().splitlines()[-1])


def java_environment(home: Path = JDK_HOME) -> dict:
    return dict(os.environ, JAVA_HOME=str(home), PATH=f"{home / 'bin'}:{os.environ['PATH']}")


def folder_bytes(folder: Path) -> int:
    return sum(path.stat().st_size for path in folder.rglob("*") if path.is_file() and not path.is_symlink())


def non_blank_lines(files: list[Path]) -> int:
    return sum(1 for path in files for line in path.read_text().splitlines() if line.strip())


def process_memory(pid: int) -> dict:
    fields = {}
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ("VmRSS", "VmHWM"):
            fields[key] = int(value.split()[0]) * 1024
    return {"rss_mib": fields["VmRSS"] / MIB, "peak_rss_mib": fields["VmHWM"] / MIB}


def process_cpu_seconds(pid: int) -> float:
    # Fields after the command name: utime and stime are the 12th and 13th, in clock ticks.
    after_name = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(after_name[11]) + int(after_name[12])) / CLOCK_TICKS_PER_SECOND


def stderr_file(name: str):
    """Server output goes to a file: an unread pipe fills up and freezes the process mid-benchmark."""
    return open(SCRATCH_FOLDER / f"{name}.stderr.log", "w")


def wait_for_health(port: int, process: subprocess.Popen) -> float:
    start = time.perf_counter()
    url = f"http://{LOOPBACK}:{port}/health"
    while time.perf_counter() - start < STARTUP_TIMEOUT_SECONDS:
        if process.poll() is not None:
            output = Path(process.stderr.name).read_text()[-1500:] if process.stderr else ""
            raise RuntimeError(f"exited during startup: {output}")
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if response.status == 200:
                    return time.perf_counter() - start
        except OSError:
            time.sleep(HEALTH_POLL_SECONDS)
    raise TimeoutError(f"no healthy response on port {port}")


def stop(process: subprocess.Popen) -> None:
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=20)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


# ----------------------------------------------------------------------------------------------
# 0. Builds and footprint
# ----------------------------------------------------------------------------------------------

def build_rust() -> dict:
    run([str(CARGO), "clean"], RUST_FOLDER)
    clean_seconds = timed([str(CARGO), "build", "--release", "--offline"], RUST_FOLDER)
    (RUST_FOLDER / "src" / "safetensors.rs").touch()
    incremental_release_seconds = timed([str(CARGO), "build", "--release", "--offline"], RUST_FOLDER)
    run([str(CARGO), "build", "--offline"], RUST_FOLDER)  # a debug build to iterate against
    (RUST_FOLDER / "src" / "safetensors.rs").touch()
    incremental_debug_seconds = timed([str(CARGO), "build", "--offline"], RUST_FOLDER)
    stripped = SCRATCH_FOLDER / "bakeoff-rust-stripped"
    run(["strip", "-o", str(stripped), str(RUST_BINARY)])
    return {
        "clean_build_seconds": clean_seconds,
        "incremental_build_seconds": incremental_release_seconds,
        "incremental_dev_build_seconds": incremental_debug_seconds,
        "shippable": "one binary",
        "shippable_mib": stripped.stat().st_size / MIB,
    }


def jlink_runtime_mib() -> tuple[float, str]:
    """The smallest Java runtime this jar needs: jdeps finds its modules, jlink builds just those."""
    modules = run([str(JDK_HOME / "bin" / "jdeps"), "--ignore-missing-deps", "--print-module-deps", "--multi-release", "25", str(JAVA_JAR)]).stdout.strip()
    modules = ",".join(sorted(set(modules.split(",")) | {"jdk.httpserver", "jdk.crypto.ec"}))
    runtime = SCRATCH_FOLDER / "jlink-runtime"
    shutil.rmtree(runtime, ignore_errors=True)
    run([str(JDK_HOME / "bin" / "jlink"), "--add-modules", modules, "--strip-debug", "--no-header-files",
         "--no-man-pages", "--compress", "zip-6", "--output", str(runtime)])
    return folder_bytes(runtime) / MIB, modules


def build_java() -> dict:
    environment = java_environment()
    clean_seconds = timed([str(MAVEN), "-q", "-B", "-o", "clean", "package"], JAVA_FOLDER, environment)
    (JAVA_FOLDER / "src" / "main" / "java" / "rastai" / "bakeoff" / "Safetensors.java").touch()
    incremental_seconds = timed([str(MAVEN), "-q", "-B", "-o", "package"], JAVA_FOLDER, environment)
    (JAVA_FOLDER / "src" / "main" / "java" / "rastai" / "bakeoff" / "Safetensors.java").touch()
    incremental_compile_seconds = timed([str(MAVEN), "-q", "-B", "-o", "compile"], JAVA_FOLDER, environment)
    runtime_mib, modules = jlink_runtime_mib()
    jar_mib = JAVA_JAR.stat().st_size / MIB
    return {
        "clean_build_seconds": clean_seconds,
        "incremental_build_seconds": incremental_seconds,
        "incremental_dev_build_seconds": incremental_compile_seconds,
        "shippable": "jar + jlink runtime",
        "shippable_mib": jar_mib + runtime_mib,
        "jar_mib": jar_mib,
        "jlink_runtime_mib": runtime_mib,
        "runtime_modules": modules,
    }


def record_native_image_configuration(model_folder: Path) -> Path:
    """GraalVM needs to know ahead of time what the program reflects on, loads through JNI, or reads as a
    resource. The tracing agent records that while the program runs every command once."""
    configuration = SCRATCH_FOLDER / "native-image-config"
    shutil.rmtree(configuration, ignore_errors=True)
    graal_java = [str(GRAALVM_HOME / "bin" / "java"), f"-agentlib:native-image-agent=config-merge-dir={configuration}", NATIVE_ACCESS, "-jar", str(JAVA_JAR)]
    tokenizer = str(model_folder / "tokenizer.json")
    run(graal_java + ["safetensors-validate", str(SAFETENSORS_FOLDER)])
    run(graal_java + ["safetensors-read", str(SAFETENSORS_FOLDER / "valid_two_tensors.safetensors")])
    run(graal_java + ["tokenize-bench", tokenizer, str(FIXTURES_FOLDER / "tokenizer_corpus.json"), str(FIXTURES_FOLDER / "tokenizer_expected.json"), "1"])

    log = SCRATCH_FOLDER / "agent-gateway.sqlite"
    gateway = subprocess.Popen(graal_java + ["gateway", str(GATEWAY_PORT), f"http://{LOOPBACK}:{MOCK_UPSTREAM_PORT}", tokenizer, str(log)],
                               stdout=subprocess.DEVNULL, stderr=stderr_file("agent-gateway"))
    wait_for_health(GATEWAY_PORT, gateway)
    for _ in range(3):
        chat_once(GATEWAY_PORT, END_TO_END_QUESTION, 8)
    stop(gateway)  # the agent writes its files when the JVM exits
    return configuration


def build_native_image(model_folder: Path) -> dict:
    configuration = record_native_image_configuration(model_folder)
    command = [
        str(GRAALVM_HOME / "bin" / "native-image"), NATIVE_ACCESS, "--enable-monitoring=threaddump",
        # The JDK server reads this in a static initializer, which native-image runs at build time,
        # so setting it at runtime (as the JVM build does) is too late here.
        "-Dsun.net.httpserver.nodelay=true",
        "-H:+UnlockExperimentalVMOptions", f"-H:ConfigurationFileDirectories={configuration}",
        "-jar", str(JAVA_JAR), "-o", str(JAVA_NATIVE_BINARY),
    ]
    start = time.perf_counter()
    completed = run(command, JAVA_FOLDER, java_environment(GRAALVM_HOME), check=False)
    seconds = time.perf_counter() - start
    if completed.returncode != 0:
        return {"built": False, "build_seconds": seconds, "error": (completed.stdout + completed.stderr)[-3000:]}
    return {"built": True, "clean_build_seconds": seconds, "shippable": "one binary", "shippable_mib": JAVA_NATIVE_BINARY.stat().st_size / MIB}


def lines_of_code() -> dict:
    rust_sources = [RUST_FOLDER / "src" / name for name in ("main.rs", "safetensors.rs", "tokenize_bench.rs", "gateway.rs")]
    java_sources = sorted((JAVA_FOLDER / "src" / "main" / "java").rglob("*.java"))
    return {
        "Rust": non_blank_lines(rust_sources) - lines_used_only_by_shared_tools(),
        "Java": non_blank_lines(java_sources),
    }


def lines_used_only_by_shared_tools() -> int:
    """main.rs also serves the shared load generator and fake upstream; don't count those bits against Rust."""
    main_text = (RUST_FOLDER / "src" / "main.rs").read_text()
    return sum(1 for line in main_text.splitlines() if "mock_upstream" in line or "load::" in line or "mod load" in line)


# ----------------------------------------------------------------------------------------------
# 1 and 2. Correctness and raw speed
# ----------------------------------------------------------------------------------------------

def safetensors_checks(command: list[str], expected: dict, gemma_file: Path) -> dict:
    verdicts = run_json(command + ["safetensors-validate", str(SAFETENSORS_FOLDER)])
    disagreements = [name for name in expected if verdicts[name]["accepted"] != expected[name]["accepted"]]
    unexpected = [name for name, verdict in verdicts.items() if "UNEXPECTED" in verdict.get("error", "")]
    reads = [run_json(command + ["safetensors-read", str(gemma_file)]) for _ in range(SAFETENSORS_READ_RUNS)]
    return {
        "verdicts_matching_contract": len(expected) - len(disagreements),
        "verdict_count": len(expected),
        "disagreements": disagreements,
        "crashes": unexpected,
        "gemma_tensors": reads[-1]["tensors"],
        "gemma_crc32c": reads[-1]["crc32c"],
        "header_ms": statistics.median(read["header_ms"] for read in reads),
        "checksum_ms": statistics.median(read["read_ms"] for read in reads),
    }


def tokenizer_checks(command: list[str], model_folder: Path) -> dict:
    return run_json(command + ["tokenize-bench", str(model_folder / "tokenizer.json"), str(FIXTURES_FOLDER / "tokenizer_corpus.json"),
                               str(FIXTURES_FOLDER / "tokenizer_expected.json"), str(TOKENIZER_REPEATS)])


# ----------------------------------------------------------------------------------------------
# 3. Gateway
# ----------------------------------------------------------------------------------------------

def start_gateway(command: list[str], upstream_port: int, model_folder: Path, log: Path) -> subprocess.Popen:
    for leftover in log.parent.glob(log.name + "*"):
        leftover.unlink()
    arguments = ["gateway", str(GATEWAY_PORT), f"http://{LOOPBACK}:{upstream_port}", str(model_folder / "tokenizer.json"), str(log)]
    return subprocess.Popen(command + arguments, stdout=subprocess.DEVNULL, stderr=stderr_file(log.stem))


def chat_once(port: int, question: str, max_tokens: int, timeout: float = 120) -> str:
    body = json.dumps({"messages": [{"role": "user", "content": question}], "max_tokens": max_tokens, "stream": True}).encode()
    request = urllib.request.Request(f"http://{LOOPBACK}:{port}/v1/chat/completions", data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode()


def load(concurrency: int, seconds: float) -> dict:
    return run_json([str(RUST_BINARY), "load", f"http://{LOOPBACK}:{GATEWAY_PORT}/v1/chat/completions",
                     str(concurrency), str(seconds), str(TOKENS_PER_REQUEST)])


def measure_startup(command: list[str], model_folder: Path) -> dict:
    startup_seconds, idle_memory = [], []
    for trial in range(STARTUP_TRIALS):
        gateway = start_gateway(command, MOCK_UPSTREAM_PORT, model_folder, SCRATCH_FOLDER / f"startup-{trial}.sqlite")
        try:
            startup_seconds.append(wait_for_health(GATEWAY_PORT, gateway))
            time.sleep(0.5)  # let startup work settle before reading idle memory
            idle_memory.append(process_memory(gateway.pid)["rss_mib"])
        finally:
            stop(gateway)
    return {"startup_ms_median": 1000 * statistics.median(startup_seconds), "idle_rss_mib_median": statistics.median(idle_memory)}


def still_answering() -> bool:
    """After a load level: does the gateway still serve a normal request within a few seconds?"""
    try:
        return "[DONE]" in chat_once(GATEWAY_PORT, END_TO_END_QUESTION, 8, timeout=10)
    except OSError:
        return False


def measure_load(command: list[str], model_folder: Path) -> dict:
    log = SCRATCH_FOLDER / "load.sqlite"
    gateway = start_gateway(command, MOCK_UPSTREAM_PORT, model_folder, log)
    try:
        wait_for_health(GATEWAY_PORT, gateway)
        load(16, WARMUP_SECONDS)  # a running server is warm; for the JVM this lets the JIT compile hot paths
        levels = {}
        for concurrency in LOAD_CONCURRENCIES:
            cpu_before = process_cpu_seconds(gateway.pid)
            result = load(concurrency, LOAD_SECONDS)
            cpu_used = process_cpu_seconds(gateway.pid) - cpu_before
            result["cpu_microseconds_per_request"] = 1e6 * cpu_used / max(result["requests"], 1)
            result.update(process_memory(gateway.pid))
            result["still_answering"] = still_answering()
            levels[concurrency] = result
            print(f"[{EXPERIMENT_NAME}]   c={concurrency}: {result['requests_per_second']:.0f} req/s", flush=True)
    finally:
        stop(gateway)
    with sqlite3.connect(log) as connection:
        logged = connection.execute("SELECT count(*) FROM gateway_log").fetchone()[0]
    served = sum(level["requests"] for level in levels.values())
    return {"levels": levels, "every_request_logged": logged >= served}


def end_to_end(contestants: dict, model_folder: Path) -> dict:
    """The same question through each gateway to the real model: logged token IDs must be identical."""
    results = {}
    gguf = snapshot_path(GGUF_REPOSITORY) / GGUF_FILE
    with LlamaServer(gguf, "CUDA0", LLAMA_SERVER_PORT):
        for name, command in contestants.items():
            log = SCRATCH_FOLDER / "end-to-end.sqlite"
            gateway = start_gateway(command, LLAMA_SERVER_PORT, model_folder, log)
            try:
                wait_for_health(GATEWAY_PORT, gateway)
                stream = chat_once(GATEWAY_PORT, END_TO_END_QUESTION, END_TO_END_TOKENS)
            finally:
                stop(gateway)
            with sqlite3.connect(log) as connection:
                prompt_ids, generated_ids = connection.execute("SELECT prompt_ids, generated_ids FROM gateway_log").fetchone()
            text = "".join(json.loads(line[6:])["choices"][0]["delta"].get("content", "")
                           for line in stream.splitlines() if line.startswith("data: {"))
            results[name] = {"prompt_ids": json.loads(prompt_ids), "generated_ids": json.loads(generated_ids), "text": text}
    reference = next(iter(results.values()))
    for result in results.values():
        result["identical_to_first"] = result["prompt_ids"] == reference["prompt_ids"] and result["generated_ids"] == reference["generated_ids"]
    return results


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def build_markdown(results: dict) -> str:
    contestants = results["contestants"]
    lines = [
        "# Experiment 07: Rust vs Java for the backend",
        "",
        "Same command-line contract, idiomatic code on each side. Rust: tokio, axum, reqwest, serde_json, rusqlite, HF `tokenizers`. "
        "Java 25: JDK HttpServer on virtual threads, java.net.http, Jackson, sqlite-jdbc, DJL tokenizers (JNI to the same Rust library).",
        "",
        "## Correctness",
        "",
        "| Contestant | Safetensors verdicts | Crashes | Gemma 2 GB checksum | Tokenizer mismatches (162 cases) | End-to-end token IDs identical |",
        "|---|---|---|---|---|---|",
    ]
    for name, data in contestants.items():
        if "skipped" in data:
            lines.append(f"| {name} | skipped: {data['skipped']} | | | | |")
            continue
        safetensors, tokenizer = data["safetensors"], data["tokenizer"]
        end_to_end_result = results["end_to_end"].get(name, {})
        lines.append(
            f"| {name} | {safetensors['verdicts_matching_contract']}/{safetensors['verdict_count']} | {len(safetensors['crashes'])} | "
            f"{safetensors['gemma_crc32c']} | {tokenizer['mismatches']} | {end_to_end_result.get('identical_to_first', '—')} |"
        )

    lines += ["", "## Speed of the building blocks", "", "| Contestant | Header parse + validation | 2 GB checksum pass | Tokenizer load | Tokens/s (1 thread) |", "|---|---|---|---|---|"]
    for name, data in contestants.items():
        if "skipped" in data:
            continue
        safetensors, tokenizer = data["safetensors"], data["tokenizer"]
        lines.append(f"| {name} | {safetensors['header_ms']:.1f} ms | {safetensors['checksum_ms']:.0f} ms | {tokenizer['load_ms']:.0f} ms | {tokenizer['tokens_per_second'] / 1e6:.2f} M |")

    lines += ["", f"## Gateway under load (fake instant upstream, {TOKENS_PER_REQUEST} tokens per streamed reply, {WARMUP_SECONDS} s warm-up, {LOAD_SECONDS} s per level)", ""]
    lines += ["| Contestant | Startup | Idle memory | " + " | ".join(f"c={c} req/s (p99 ms)" for c in LOAD_CONCURRENCIES) + " | CPU µs/request (c=64) | Peak memory |",
              "|---|---|---|" + "---|" * len(LOAD_CONCURRENCIES) + "---|---|"]
    for name, data in contestants.items():
        if "skipped" in data:
            continue
        levels = data["load"]["levels"]
        cells = " | ".join(f"{levels[c]['requests_per_second']:,.0f} ({levels[c]['total_ms_p99']:.1f})" for c in LOAD_CONCURRENCIES)
        peak = max(level["peak_rss_mib"] for level in levels.values())
        lines.append(f"| {name} | {data['startup']['startup_ms_median']:.0f} ms | {data['startup']['idle_rss_mib_median']:.0f} MiB | {cells} | "
                     f"{levels[64]['cpu_microseconds_per_request']:.0f} | {peak:.0f} MiB |")

    lines += ["", "## Building and shipping", "", "| Contestant | Clean build | Incremental release build | Incremental dev build | Shippable | Size |", "|---|---|---|---|---|---|"]
    for name, build in results["builds"].items():
        if not build.get("built", True):
            lines.append(f"| {name} | failed after {build['build_seconds']:.0f} s | | | | |")
            continue
        incremental = build.get("incremental_build_seconds")
        incremental_dev = build.get("incremental_dev_build_seconds")
        lines.append(f"| {name} | {build['clean_build_seconds']:.0f} s | {f'{incremental:.1f} s' if incremental is not None else '—'} | "
                     f"{f'{incremental_dev:.1f} s' if incremental_dev is not None else '—'} | {build['shippable']} | {build['shippable_mib']:.0f} MiB |")
    lines += ["", f"Lines of code (non-blank, same features): Rust {results['lines_of_code']['Rust']}, Java {results['lines_of_code']['Java']}.", ""]

    if results["end_to_end"]:
        first = next(iter(results["end_to_end"].values()))
        lines += ["## End to end through the real model", "", f"Answer via every gateway: {first['text']!r}", ""]
    lines += ["## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    shutil.rmtree(SCRATCH_FOLDER, ignore_errors=True)
    SCRATCH_FOLDER.mkdir(parents=True)
    model_folder = snapshot_path(MODEL_REPOSITORY)
    expected_verdicts = write_safetensors_fixtures()
    write_tokenizer_fixtures(model_folder)

    print(f"[{EXPERIMENT_NAME}] building", flush=True)
    mock_upstream = subprocess.Popen([str(RUST_BINARY), "mock-upstream", str(MOCK_UPSTREAM_PORT)], stderr=stderr_file("mock-upstream"))
    try:
        builds = {"Rust": build_rust(), "Java (JVM)": build_java()}
        builds["Java, GraalVM native image"] = build_native_image(model_folder)
        contestants = dict(CONTESTANTS)
        if not builds["Java, GraalVM native image"]["built"]:
            contestants.pop("Java, GraalVM native image")

        results = {"builds": builds, "lines_of_code": lines_of_code(), "contestants": {}}
        gemma_file = model_folder / "model.safetensors"
        for name, command in contestants.items():
            print(f"[{EXPERIMENT_NAME}] {name}", flush=True)
            results["contestants"][name] = {
                "safetensors": safetensors_checks(command, expected_verdicts, gemma_file),
                "tokenizer": tokenizer_checks(command, model_folder),
                "startup": measure_startup(command, model_folder),
                "load": measure_load(command, model_folder),
            }
        if "Java, GraalVM native image" not in contestants:
            results["contestants"]["Java, GraalVM native image"] = {"skipped": "native image build failed (see builds)"}
    finally:
        stop(mock_upstream)

    print(f"[{EXPERIMENT_NAME}] end to end through the real model", flush=True)
    results["end_to_end"] = end_to_end(contestants, model_folder)
    results["environment"] = environment_fingerprint()
    shutil.rmtree(SCRATCH_FOLDER, ignore_errors=True)

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
