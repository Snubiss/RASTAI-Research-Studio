"""
Experiment 04: does llama.cpp agree with the gateway's tokenizer, and how fast is the Fast runtime?
(spec v5 §32, spike item 3; spec §18: the gateway owns tokenization)

  1. Tokenizer parity: the gateway's tokenizer (HF `tokenizers`) vs llama-server's built-in tokenizer on a
     stress corpus: every paragraph of our spec, the whole spec, source code, and hand-picked edge cases
     (scripts, emoji, whitespace, control characters, special tokens written as text). Checked both
     without and with the begin-of-text token, plus detokenization back to text.
  2. Generation parity: the same token IDs sent to llama-server (f16 and Q4_K_M GGUF) and to our
     in-house runtime, greedy. How many generated tokens agree before they diverge?
  3. Speed: llama-bench decode and prompt throughput on both GPUs, next to our Inspectable runtime's
     numbers from experiment 02, to size the gap the two-runtime design (spec §8) exists for.

Pass criterion for item 1 (fixed before the first run): 100% identical token IDs on the whole corpus.
Items 2 and 3 are measurements, not pass/fail.

Run:  uv run python experiments/04_tokenizer_and_fast_runtime.py
"""

import json
import subprocess
from pathlib import Path

import torch

from rastai_spike.gemma3 import load_model
from rastai_spike.generation import generate_greedy, load_stop_token_ids
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.llama_server import BENCH_BINARY, LlamaServer, library_environment
from rastai_spike.prompts import BEGIN_TEXT_TOKEN, build_prompts, load_tokenizer
from rastai_spike.reporting import RESULTS_FOLDER, environment_fingerprint, write_results
from rastai_spike.tokenizer_corpus import EDGE_CASES, build_corpus

EXPERIMENT_NAME = "04_tokenizer_and_fast_runtime"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
GGUF_REPOSITORY = "ggml-org/gemma-3-1b-it-GGUF"
GGUF_FILES = {"f16": "gemma-3-1b-it-f16.gguf", "Q4_K_M": "gemma-3-1b-it-Q4_K_M.gguf"}
SERVER_PORT = 18089
SERVER_DEVICE = "CUDA0"
TORCH_DEVICE = "cuda:0"
BENCH_DEVICES = {"CUDA0": "RTX 5080", "CUDA1": "RTX 3090"}
BENCH_PROMPT_TOKENS = 512
BENCH_GENERATED_TOKENS = 128
GENERATED_TOKEN_COUNT = 64
MISMATCH_EXAMPLES_KEPT = 10

# ----------------------------------------------------------------------------------------------
# 1. Tokenizer parity
# ----------------------------------------------------------------------------------------------

def first_difference(expected: list, actual: list) -> int | None:
    for index, (left, right) in enumerate(zip(expected, actual)):
        if left != right:
            return index
    return None if len(expected) == len(actual) else min(len(expected), len(actual))


def tokenizer_parity(server: LlamaServer, tokenizer, corpus: dict[str, str]) -> dict:
    begin_text_id = tokenizer.token_to_id(BEGIN_TEXT_TOKEN)
    checks = {"without begin-of-text": [], "with begin-of-text": [], "detokenize": []}
    total_tokens = 0

    for name, text in corpus.items():
        ours = tokenizer.encode(text, add_special_tokens=False).ids
        total_tokens += len(ours)
        theirs = server.tokenize(text, add_special=False)
        theirs_with_begin = server.tokenize(text, add_special=True)

        record_comparison(checks["without begin-of-text"], name, ours, theirs, tokenizer)
        record_comparison(checks["with begin-of-text"], name, [begin_text_id] + ours, theirs_with_begin, tokenizer)
        record_text_comparison(checks["detokenize"], name, tokenizer.decode(ours, skip_special_tokens=False), server.detokenize(ours))

    return {
        "cases": len(corpus),
        "tokens": total_tokens,
        "checks": {
            check: {"mismatches": len(mismatches), "examples": mismatches[:MISMATCH_EXAMPLES_KEPT]}
            for check, mismatches in checks.items()
        },
    }


def record_comparison(mismatches: list, name: str, ours: list[int], theirs: list[int], tokenizer) -> None:
    index = first_difference(ours, theirs)
    if index is None:
        return
    window = slice(max(0, index - 3), index + 4)
    mismatches.append({
        "case": name,
        "first_difference_at": index,
        "ours": ours[window],
        "llama_cpp": theirs[window],
        "ours_pieces": [tokenizer.id_to_token(token_id) for token_id in ours[window]],
        "llama_cpp_pieces": [tokenizer.id_to_token(token_id) for token_id in theirs[window]],
        "lengths": [len(ours), len(theirs)],
    })


def record_text_comparison(mismatches: list, name: str, ours: str, theirs: str) -> None:
    if ours == theirs:
        return
    index = first_difference(list(ours), list(theirs))
    mismatches.append({"case": name, "first_difference_at": index, "ours": repr(ours[max(0, index - 10) : index + 10]), "llama_cpp": repr(theirs[max(0, index - 10) : index + 10])})


# ----------------------------------------------------------------------------------------------
# 2. Generation parity
# ----------------------------------------------------------------------------------------------

def matching_prefix(expected: list[int], actual: list[int]) -> int:
    difference = first_difference(expected, actual)
    return min(len(expected), len(actual)) if difference is None else difference


def our_generations(model_folder, prompts, dtype) -> dict[str, list[int]]:
    model = load_model(model_folder, TORCH_DEVICE, dtype)
    stop_token_ids = load_stop_token_ids(model_folder)
    generations = {name: generate_greedy(model, ids, GENERATED_TOKEN_COUNT, stop_token_ids) for name, ids in prompts.items()}
    del model
    torch.cuda.empty_cache()
    return generations


def generation_parity(server: LlamaServer, prompts, ours_by_precision: dict, tokenizer) -> dict:
    results = {}
    for name, prompt_ids in prompts.items():
        theirs = server.complete_greedy(prompt_ids, GENERATED_TOKEN_COUNT)["generated_ids"]
        results[name] = {
            f"matching_tokens_vs_ours_{precision}": matching_prefix(ours[name], theirs)
            for precision, ours in ours_by_precision.items()
        }
        results[name]["generated_tokens"] = len(theirs)
        results[name]["llama_cpp_text"] = tokenizer.decode(theirs, skip_special_tokens=True)[:200]
    return results


# ----------------------------------------------------------------------------------------------
# 3. Speed
# ----------------------------------------------------------------------------------------------

def bench(gguf_path: Path, device: str) -> dict:
    command = [
        str(BENCH_BINARY), "-m", str(gguf_path), "-dev", device, "-ngl", "999",
        "-p", str(BENCH_PROMPT_TOKENS), "-n", str(BENCH_GENERATED_TOKENS), "-o", "json",
    ]
    output = subprocess.run(command, env=library_environment(), capture_output=True, text=True, check=True).stdout
    results = json.loads(output)
    by_kind = {}
    for row in results:
        kind = "prompt" if row["n_prompt"] > 0 else "decode"
        by_kind[f"{kind}_tokens_per_second"] = row["avg_ts"]
    return by_kind


def inspectable_runtime_reference() -> dict:
    """Our runtime's decode speed from experiment 02 (chat prompt, CUDA graph, panel off and fused), if it ran."""
    path = RESULTS_FOLDER / "02_live_lens_overhead.json"
    if not path.exists():
        return {}
    reference = {}
    for device_name, device_result in json.loads(path.read_text())["devices"].items():
        runs = device_result["runs"]
        reference[device_name] = {
            "panel_off": runs["chat / cuda graph / off"]["tokens_per_second"],
            "panel_on_fused": runs["chat / cuda graph / fused"]["tokens_per_second"],
        }
    return reference


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def build_markdown(results: dict) -> str:
    parity = results["tokenizer_parity"]
    lines = [
        "# Experiment 04: tokenizer parity and the Fast runtime",
        "",
        f"llama.cpp build {results['llama_cpp_build']}. Gateway tokenizer: HF `tokenizers` {results['tokenizers_version']}.",
        "",
        f"## 1. Tokenizer parity: **{'PASS' if results['tokenizer_pass'] else 'FAIL'}**",
        "",
        f"{parity['cases']} cases, {parity['tokens']:,} tokens (every spec paragraph, the whole spec, source code, {len(EDGE_CASES)} edge cases).",
        "",
        "| Check | Mismatching cases |",
        "|---|---|",
        *[f"| {check} | {data['mismatches']} |" for check, data in parity["checks"].items()],
        "",
    ]
    for check, data in parity["checks"].items():
        for example in data["examples"]:
            lines.append(f"- {check} / {example['case']}: first difference at {example['first_difference_at']}: "
                         f"ours {example.get('ours_pieces', example['ours'])} vs llama.cpp {example.get('llama_cpp_pieces', example['llama_cpp'])}")
    lines += ["", f"## 2. Generation parity ({GENERATED_TOKEN_COUNT} greedy tokens from identical token IDs)", ""]
    for gguf_name, per_prompt in results["generation_parity"].items():
        lines += [f"**{gguf_name} GGUF on the RTX 5080**", "", "| Prompt | Tokens agreeing with ours (bf16) | with ours (fp32) |", "|---|---|---|"]
        for prompt_name, row in per_prompt.items():
            lines.append(f"| {prompt_name} | {row['matching_tokens_vs_ours_bf16']}/{row['generated_tokens']} | {row['matching_tokens_vs_ours_fp32']}/{row['generated_tokens']} |")
        lines.append("")
    lines += ["## 3. Speed", "", "| Runtime | GPU | Decode tokens/s | Prompt tokens/s |", "|---|---|---|---|"]
    for gguf_name, per_device in results["speed"].items():
        for device, numbers in per_device.items():
            lines.append(f"| llama.cpp {gguf_name} | {BENCH_DEVICES[device]} | {numbers['decode_tokens_per_second']:.0f} | {numbers['prompt_tokens_per_second']:.0f} |")
    for device_name, numbers in results["inspectable_runtime_reference"].items():
        lines.append(f"| Rastai Inspectable bf16, panel off (exp. 02) | {device_name} | {numbers['panel_off']:.0f} | — |")
        lines.append(f"| Rastai Inspectable bf16, full panel (exp. 02) | {device_name} | {numbers['panel_on_fused']:.0f} | — |")
    lines += ["", "## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    import tokenizers

    model_folder = snapshot_path(MODEL_REPOSITORY)
    gguf_folder = snapshot_path(GGUF_REPOSITORY)
    tokenizer = load_tokenizer(model_folder)
    prompts = build_prompts(tokenizer)

    print(f"[{EXPERIMENT_NAME}] our runtime: reference generations", flush=True)
    ours_by_precision = {
        "bf16": our_generations(model_folder, prompts, torch.bfloat16),
        "fp32": our_generations(model_folder, prompts, torch.float32),
    }

    results = {"tokenizers_version": tokenizers.__version__, "generation_parity": {}, "speed": {}}
    for gguf_name, file_name in GGUF_FILES.items():
        print(f"[{EXPERIMENT_NAME}] llama-server {gguf_name}", flush=True)
        with LlamaServer(gguf_folder / file_name, SERVER_DEVICE, SERVER_PORT) as server:
            if gguf_name == "f16":  # the tokenizer lives in every GGUF; checking one is enough
                results["llama_cpp_build"] = server.get("/props").get("build_info", "unknown")
                results["tokenizer_parity"] = tokenizer_parity(server, tokenizer, build_corpus())
            results["generation_parity"][gguf_name] = generation_parity(server, prompts, ours_by_precision, tokenizer)
        print(f"[{EXPERIMENT_NAME}] llama-bench {gguf_name}", flush=True)
        results["speed"][gguf_name] = {device: bench(gguf_folder / file_name, device) for device in BENCH_DEVICES}

    results["tokenizer_pass"] = all(check["mismatches"] == 0 for check in results["tokenizer_parity"]["checks"].values())
    results["inspectable_runtime_reference"] = inspectable_runtime_reference()
    results["environment"] = environment_fingerprint()

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
