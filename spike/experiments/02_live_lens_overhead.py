"""
Experiment 02: what does the live Psychology panel cost? (spec v5 §32, spike item 2)

Measures decode speed (tokens/s) and VRAM for Gemma 3 1B in bf16 with the live panel off and on:
  - devices: RTX 5080 capped at 8 GiB (the First Run budget), and RTX 3090 uncapped;
  - contexts: the 19-token chat prompt and the 1,600-token long prompt;
  - execution: a plain Python loop ("eager") and one decode step replayed as a CUDA graph;
  - panel: off / naive / batched (CTO rules) / fused (lens shares the unembedding with the output).

The panel computes, per token: confidence and top-10, a logit lens over all 26 layers, the newest token's
top attended positions per head and layer, and the top-16 features of real Gemma Scope 2 SAEs on 3 layers.

It also checks the readouts are real: the SAEs should fire roughly as many features as they were trained
for, and reconstruct the residual stream well, which only happens if our residual stream is correct.

Run:  uv run python experiments/02_live_lens_overhead.py
"""

import gc
import time

import torch

from rastai_spike.decoding import LENS_BATCHED, LENS_FUSED, LENS_NAIVE, LENS_OFF, StaticDecoder
from rastai_spike.gemma3 import load_model
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.lenses import LensCollector, unpack
from rastai_spike.prompts import build_prompts, load_tokenizer
from rastai_spike.reporting import device_label, environment_fingerprint, write_results
from rastai_spike.sae import gemma_scope_folder, load_sparse_autoencoder

EXPERIMENT_NAME = "02_live_lens_overhead"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
SAE_REPOSITORY = "google/gemma-scope-2-1b-it"
SAE_LAYERS = (7, 13, 22)
DEVICE_MEMORY_CAPS_GIB = {"cuda:0": 8.0, "cuda:1": None}  # 5080 held to the First Run's 8 GiB budget
CONTEXTS = ("chat", "long")
GENERATED_TOKEN_COUNT = 256
WARMUP_TOKEN_COUNT = 8  # untimed; includes CUDA graph capture
MATCH_CHECK_TOKEN_COUNT = 64  # compare the first N generated tokens against the panel-off baseline
SAMPLE_READOUT_TOKENS = 4
RUNS = [  # (use_cuda_graph, lens_mode)
    (False, LENS_OFF), (False, LENS_NAIVE), (False, LENS_BATCHED), (False, LENS_FUSED),
    (True, LENS_OFF), (True, LENS_BATCHED), (True, LENS_FUSED),
]
GIB = 1024**3


# ----------------------------------------------------------------------------------------------
# Readout sanity
# ----------------------------------------------------------------------------------------------

@torch.inference_mode()
def sae_sanity(model, autoencoders, prompt_ids, device) -> list[dict]:
    """On real text: how many features fire per token (vs the SAE's trained L0), and how much of the
    residual stream's variance the reconstruction misses (fraction of variance unexplained)."""
    collector = AllPositionsCollector()
    model(torch.tensor([prompt_ids], device=device), observer=collector)

    report = []
    for autoencoder in autoencoders:
        residual = collector.residuals[autoencoder.layer_index][0, 1:].float()  # skip <bos>, an outlier position
        features = autoencoder.encode(residual)
        reconstruction = autoencoder.decode(features)
        unexplained = ((residual - reconstruction) ** 2).sum() / ((residual - residual.mean(0)) ** 2).sum()
        report.append({
            "layer": autoencoder.layer_index,
            "trained_active_features": autoencoder.target_active_count,
            "measured_active_features": (features > 0).sum(-1).float().mean().item(),
            "fraction_of_variance_unexplained": unexplained.item(),
        })
    return report


class AllPositionsCollector:
    def __init__(self):
        self.residuals = {}

    def __call__(self, point_name, tensor):
        if point_name.endswith(".residual_out"):
            self.residuals[int(point_name.split(".")[1])] = tensor


def describe_readout(flat_packet, tokenizer, config, sae_count) -> dict:
    readout = unpack(flat_packet[1:], config.layer_count, config.query_head_count, sae_count)
    chosen = int(flat_packet[0])
    lens_top1 = [int(token_id) for token_id in readout["lens_ids"][:, 0]]
    first_matching_layer = next((layer for layer, token_id in enumerate(lens_top1) if token_id == chosen), None)
    return {
        "chosen_token": tokenizer.decode([chosen]),
        "confidence_entropy": round(readout["final_entropy"].item(), 3),
        "top3": [
            (tokenizer.decode([int(token_id)]), round(probability.item(), 3))
            for token_id, probability in zip(readout["final_ids"][:3], readout["final_probabilities"][:3])
        ],
        "first_layer_whose_lens_agrees": first_matching_layer,
        "lens_top1_every_5th_layer": {layer: tokenizer.decode([lens_top1[layer]]) for layer in range(0, config.layer_count, 5)},
        "sae_active_counts": [int(readout[f"sae_{index}_active_count"].item()) for index in range(sae_count)],
    }


# ----------------------------------------------------------------------------------------------
# Timing one configuration
# ----------------------------------------------------------------------------------------------

def measure_decode(model, prompt_ids, use_cuda_graph, lens_mode, autoencoders, device) -> tuple[dict, list[torch.Tensor]]:
    free_memory()
    torch.cuda.reset_peak_memory_stats(device)
    weights_bytes = torch.cuda.memory_allocated(device)

    max_length = len(prompt_ids) + WARMUP_TOKEN_COUNT + GENERATED_TOKEN_COUNT + 1
    decoder = StaticDecoder(model, max_length, lens_mode, autoencoders, use_cuda_graph)

    prefill_start = time.perf_counter()
    first_token = decoder.prefill(prompt_ids)
    torch.cuda.synchronize(device)
    prefill_seconds = time.perf_counter() - prefill_start

    packets = []
    for _ in range(WARMUP_TOKEN_COUNT):
        packets.append(decoder.step().clone())

    decode_start = time.perf_counter()
    for _ in range(GENERATED_TOKEN_COUNT):
        packets.append(decoder.step().clone())
    decode_seconds = time.perf_counter() - decode_start

    peak_bytes = torch.cuda.max_memory_allocated(device)
    result = {
        "tokens_per_second": GENERATED_TOKEN_COUNT / decode_seconds,
        "milliseconds_per_token": 1000 * decode_seconds / GENERATED_TOKEN_COUNT,
        "prefill_milliseconds": 1000 * prefill_seconds,
        "peak_memory_gib": peak_bytes / GIB,
        "working_memory_gib": (peak_bytes - weights_bytes) / GIB,
        "packet_bytes_per_token": packets[-1].numel() * 4,
        "generated_ids": [first_token] + [int(packet[0]) for packet in packets[: MATCH_CHECK_TOKEN_COUNT - 1]],
    }
    del decoder
    return result, packets


def free_memory() -> None:
    gc.collect()
    torch.cuda.empty_cache()


def matching_prefix_length(baseline: list[int], candidate: list[int]) -> int:
    length = 0
    for expected, actual in zip(baseline, candidate):
        if expected != actual:
            break
        length += 1
    return length


# ----------------------------------------------------------------------------------------------
# One device
# ----------------------------------------------------------------------------------------------

def apply_memory_cap(device: str, cap_gib: float | None) -> None:
    if cap_gib is not None:
        total = torch.cuda.get_device_properties(device).total_memory
        torch.cuda.set_per_process_memory_fraction(cap_gib * GIB / total, device)


def run_device(device, model_folder, sae_snapshot, prompts, tokenizer) -> dict:
    torch.cuda.set_device(device)
    apply_memory_cap(device, DEVICE_MEMORY_CAPS_GIB[device])

    model = load_model(model_folder, device, torch.bfloat16)
    autoencoders = [load_sparse_autoencoder(gemma_scope_folder(sae_snapshot, layer), device) for layer in SAE_LAYERS]
    device_result = {
        "memory_cap_gib": DEVICE_MEMORY_CAPS_GIB[device],
        "sae_sanity": sae_sanity(model, autoencoders, prompts["long"], device),
        "runs": {},
    }

    for context in CONTEXTS:
        baseline_ids = None
        for use_cuda_graph, lens_mode in RUNS:
            run_name = f"{context} / {'cuda graph' if use_cuda_graph else 'eager'} / {lens_mode}"
            print(f"[{EXPERIMENT_NAME}] {device}: {run_name}", flush=True)
            result, packets = measure_decode(model, prompts[context], use_cuda_graph, lens_mode, autoencoders, device)

            baseline_ids = baseline_ids or result["generated_ids"]
            result["tokens_matching_baseline"] = matching_prefix_length(baseline_ids, result["generated_ids"])
            if context == "chat" and use_cuda_graph and lens_mode == LENS_FUSED:
                device_result["sample_readouts"] = [
                    describe_readout(packet, tokenizer, model.config, len(autoencoders))
                    for packet in packets[:SAMPLE_READOUT_TOKENS]
                ]
            device_result["runs"][run_name] = result

    del model, autoencoders
    free_memory()
    return device_result


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def build_markdown(results: dict) -> str:
    lines = [
        "# Experiment 02: live Psychology panel overhead",
        "",
        f"Gemma 3 1B-it in bf16, greedy decoding of {GENERATED_TOKEN_COUNT} tokens after {WARMUP_TOKEN_COUNT} warm-up tokens.",
        f"Panel per token: entropy + top-{10}, logit lens on all layers, top attended positions per head and layer, "
        f"top-16 Gemma Scope 2 SAE features on layers {list(SAE_LAYERS)} (16k wide, fp32).",
        "",
    ]
    for device_name, device_result in results["devices"].items():
        cap = device_result["memory_cap_gib"]
        lines += [f"## {device_name}" + (f" (capped at {cap:g} GiB)" if cap else ""), ""]
        lines += ["| Context / execution / panel | tokens/s | ms/token | vs panel off | working VRAM GiB | peak VRAM GiB | packet bytes | tokens matching baseline |",
                  "|---|---|---|---|---|---|---|---|"]
        for run_name, run in device_result["runs"].items():
            context, execution, _ = run_name.split(" / ")
            baseline = device_result["runs"][f"{context} / {execution} / off"]
            overhead = run["milliseconds_per_token"] / baseline["milliseconds_per_token"] - 1
            lines.append(
                f"| {run_name} | {run['tokens_per_second']:.1f} | {run['milliseconds_per_token']:.2f} | "
                f"{'—' if run is baseline else f'{overhead:+.0%}'} | {run['working_memory_gib']:.2f} | "
                f"{run['peak_memory_gib']:.2f} | {run['packet_bytes_per_token']} | "
                f"{run['tokens_matching_baseline']}/{MATCH_CHECK_TOKEN_COUNT} |"
            )
        lines += ["", "SAE sanity on the long prompt (does our residual stream look like what the SAEs were trained on?):", "",
                  "| Layer | Trained L0 | Measured L0 | Variance unexplained |", "|---|---|---|---|"]
        for row in device_result["sae_sanity"]:
            lines.append(f"| {row['layer']} | {row['trained_active_features']} | {row['measured_active_features']:.1f} | {row['fraction_of_variance_unexplained']:.3f} |")
        lines += ["", "First generated tokens with the panel on (chat prompt, fused, CUDA graph):", ""]
        for readout in device_result.get("sample_readouts", []):
            lines.append(
                f"- **{readout['chosen_token']!r}**: entropy {readout['confidence_entropy']}, top-3 {readout['top3']}, "
                f"logit lens first agrees at layer {readout['first_layer_whose_lens_agrees']}, "
                f"lens top-1 by layer {readout['lens_top1_every_5th_layer']}, SAE active features {readout['sae_active_counts']}"
            )
        lines.append("")

    lines += ["## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    model_folder = snapshot_path(MODEL_REPOSITORY)
    sae_snapshot = snapshot_path(SAE_REPOSITORY)
    tokenizer = load_tokenizer(model_folder)
    prompts = build_prompts(tokenizer)

    results = {"model": MODEL_REPOSITORY, "environment": environment_fingerprint(), "devices": {}}
    for device in DEVICE_MEMORY_CAPS_GIB:
        results["devices"][device_label(device)] = run_device(device, model_folder, sae_snapshot, prompts, tokenizer)

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
