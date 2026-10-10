"""
Experiment 01: does the in-house Gemma 3 1B match Hugging Face? (spec v5 §32, spike item 1)

For every GPU, compares our runtime with transformers on the same weights and token IDs:
  1. fp32 logits, with both of our attention paths (SDPA and eager), against the fp32 reference.
  2. bf16 drift: how far each implementation's bf16 logits move from the fp32 reference.
     Ours must drift no more than the reference's own bf16 run does.
  3. Greedy generation in fp32: the generated tokens must be identical.
  4. KV-cache consistency: decoding token by token must give the same logits as one full pass.

The pass criteria below were fixed before the first run.

Run:  uv run python experiments/01_gemma_parity.py
"""

import gc
import time

import torch
import torch.nn.functional as F
from transformers import AutoModelForCausalLM, GenerationConfig

from rastai_spike.gemma3 import CAPTURE_FULL_ATTENTION, CAPTURE_NO_ATTENTION, KeyValueCache, load_model
from rastai_spike.generation import generate_greedy, load_stop_token_ids
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.prompts import build_prompts, load_tokenizer
from rastai_spike.reporting import device_label, environment_fingerprint, write_results

EXPERIMENT_NAME = "01_gemma_parity"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
DEVICES = ["cuda:0", "cuda:1"]
GENERATED_TOKEN_COUNT = 64
CACHE_CHECK_DECODED_TOKENS = 64  # the last N prompt tokens are fed one at a time through the cache
COMPARISON_CHUNK_SIZE = 256  # positions per chunk when comparing [positions, 262144] logit tables

# Pass criteria (fixed before the first run)
FP32_MAX_ABS_LOGIT_DIFF_LIMIT = 5e-3
FP32_MEAN_KL_LIMIT = 1e-6
FP32_TOP1_AGREEMENT_REQUIRED = 1.0
BF16_DRIFT_RATIO_LIMIT = 1.25  # our bf16 drift from fp32 ÷ the reference's bf16 drift from fp32
BF16_TOP1_MARGIN = 0.01  # our bf16 top-1 agreement with fp32 may trail the reference's by at most 1 point
CACHE_MAX_ABS_LOGIT_DIFF_LIMIT = 5e-3


# ----------------------------------------------------------------------------------------------
# Comparing logits
# ----------------------------------------------------------------------------------------------

def compare_logits(reference: torch.Tensor, candidate: torch.Tensor, device: str) -> dict:
    """Both tensors are [positions, vocabulary] on the CPU; the work happens on `device`, a chunk at a time."""
    max_abs_diff, total_abs_diff, top1_matches, total_kl = 0.0, 0.0, 0, 0.0
    position_count, vocabulary_size = reference.shape

    for start in range(0, position_count, COMPARISON_CHUNK_SIZE):
        reference_chunk = reference[start : start + COMPARISON_CHUNK_SIZE].to(device, torch.float32)
        candidate_chunk = candidate[start : start + COMPARISON_CHUNK_SIZE].to(device, torch.float32)

        difference = (reference_chunk - candidate_chunk).abs()
        max_abs_diff = max(max_abs_diff, difference.max().item())
        total_abs_diff += difference.double().sum().item()
        top1_matches += (reference_chunk.argmax(-1) == candidate_chunk.argmax(-1)).sum().item()

        reference_log_probs = F.log_softmax(reference_chunk, dim=-1)
        candidate_log_probs = F.log_softmax(candidate_chunk, dim=-1)
        kl_per_position = (reference_log_probs.exp() * (reference_log_probs - candidate_log_probs)).sum(-1)
        total_kl += kl_per_position.double().sum().item()

    return {
        "max_abs_logit_diff": max_abs_diff,
        "mean_abs_logit_diff": total_abs_diff / (position_count * vocabulary_size),
        "top1_agreement": top1_matches / position_count,
        "mean_kl": total_kl / position_count,
    }


# ----------------------------------------------------------------------------------------------
# Running each implementation
# ----------------------------------------------------------------------------------------------

def free_gpu_memory() -> None:
    gc.collect()
    torch.cuda.empty_cache()


def load_reference(model_folder, device: str, dtype: torch.dtype):
    model = AutoModelForCausalLM.from_pretrained(model_folder, dtype=dtype, attn_implementation="eager")
    return model.to(device).eval()


@torch.inference_mode()
def reference_logits(model, prompt_ids: list[int], device: str) -> torch.Tensor:
    return model(input_ids=torch.tensor([prompt_ids], device=device)).logits[0].cpu()


@torch.inference_mode()
def reference_generate(model, prompt_ids: list[int], device: str, stop_token_ids: set[int]) -> list[int]:
    settings = GenerationConfig(
        do_sample=False,
        max_new_tokens=GENERATED_TOKEN_COUNT,
        eos_token_id=sorted(stop_token_ids),
        pad_token_id=0,
    )
    input_ids = torch.tensor([prompt_ids], device=device)
    output = model.generate(input_ids, attention_mask=torch.ones_like(input_ids), generation_config=settings)
    return output[0, len(prompt_ids) :].tolist()


@torch.inference_mode()
def rastai_logits(model, prompt_ids: list[int], device: str, use_eager: bool) -> torch.Tensor:
    capture = CAPTURE_FULL_ATTENTION if use_eager else CAPTURE_NO_ATTENTION
    return model(torch.tensor([prompt_ids], device=device), attention_capture=capture)[0].cpu()


@torch.inference_mode()
def rastai_cached_logits(model, prompt_ids: list[int], device: str) -> tuple[torch.Tensor, int]:
    """Prefill all but the last N tokens, then feed those N one at a time. Returns their logits and N."""
    decoded_count = min(CACHE_CHECK_DECODED_TOKENS, len(prompt_ids) - 1)
    split = len(prompt_ids) - decoded_count
    weight = model.token_embedding.weight
    cache = KeyValueCache(model.config, 1, len(prompt_ids), device, weight.dtype)

    model(torch.tensor([prompt_ids[:split]], device=device), cache)
    step_logits = [
        model(torch.tensor([[token_id]], device=device), cache)[0, -1].cpu()
        for token_id in prompt_ids[split:]
    ]
    return torch.stack(step_logits), decoded_count


def timed_load(loader, *arguments) -> tuple[object, float]:
    start = time.perf_counter()
    model = loader(*arguments)
    torch.cuda.synchronize()
    return model, time.perf_counter() - start


# ----------------------------------------------------------------------------------------------
# One GPU, start to finish
# ----------------------------------------------------------------------------------------------

def run_fp32_checks(model_folder, device, prompts, stop_token_ids) -> tuple[dict, dict]:
    """Returns (results, fp32 reference logits per prompt) so the bf16 checks can measure drift."""
    results = {"prompts": {}}

    reference, results["reference_load_seconds"] = timed_load(load_reference, model_folder, device, torch.float32)
    reference_outputs = {name: reference_logits(reference, ids, device) for name, ids in prompts.items()}
    reference_tokens = {name: reference_generate(reference, ids, device, stop_token_ids) for name, ids in prompts.items()}
    del reference
    free_gpu_memory()

    ours, results["rastai_load_seconds"] = timed_load(load_model, model_folder, device, torch.float32)
    for name, ids in prompts.items():
        sdpa_logits = rastai_logits(ours, ids, device, use_eager=False)
        eager_logits = rastai_logits(ours, ids, device, use_eager=True)
        cached_logits, decoded_count = rastai_cached_logits(ours, ids, device)
        our_tokens = generate_greedy(ours, ids, GENERATED_TOKEN_COUNT, stop_token_ids)

        results["prompts"][name] = {
            "token_count": len(ids),
            "reference_max_abs_logit": reference_outputs[name].abs().max().item(),
            "sdpa_vs_reference": compare_logits(reference_outputs[name], sdpa_logits, device),
            "eager_vs_reference": compare_logits(reference_outputs[name], eager_logits, device),
            "cache_vs_full_pass": compare_logits(sdpa_logits[-decoded_count:], cached_logits, device),
            "generation_identical": our_tokens == reference_tokens[name],
            "generated_token_count": len(our_tokens),
            "rastai_generated_ids": our_tokens,
            "reference_generated_ids": reference_tokens[name],
        }
    del ours
    free_gpu_memory()
    return results, reference_outputs


def run_bf16_checks(model_folder, device, prompts, fp32_reference) -> dict:
    results = {"prompts": {}}

    reference, _ = timed_load(load_reference, model_folder, device, torch.bfloat16)
    reference_bf16 = {name: reference_logits(reference, ids, device) for name, ids in prompts.items()}
    del reference
    free_gpu_memory()

    ours, _ = timed_load(load_model, model_folder, device, torch.bfloat16)
    for name, ids in prompts.items():
        our_bf16 = rastai_logits(ours, ids, device, use_eager=False)
        results["prompts"][name] = {
            "reference_bf16_drift": compare_logits(fp32_reference[name], reference_bf16[name], device),
            "rastai_bf16_drift": compare_logits(fp32_reference[name], our_bf16, device),
            "rastai_vs_reference_bf16": compare_logits(reference_bf16[name], our_bf16, device),
        }
    del ours
    free_gpu_memory()
    return results


# ----------------------------------------------------------------------------------------------
# Verdicts and the report
# ----------------------------------------------------------------------------------------------

def fp32_comparison_passes(metrics: dict) -> bool:
    return (
        metrics["max_abs_logit_diff"] <= FP32_MAX_ABS_LOGIT_DIFF_LIMIT
        and metrics["mean_kl"] <= FP32_MEAN_KL_LIMIT
        and metrics["top1_agreement"] >= FP32_TOP1_AGREEMENT_REQUIRED
    )


def bf16_drift_passes(prompt_result: dict) -> bool:
    ours, reference = prompt_result["rastai_bf16_drift"], prompt_result["reference_bf16_drift"]
    drift_ratio = ours["mean_abs_logit_diff"] / reference["mean_abs_logit_diff"]
    prompt_result["drift_ratio"] = drift_ratio
    return drift_ratio <= BF16_DRIFT_RATIO_LIMIT and ours["top1_agreement"] >= reference["top1_agreement"] - BF16_TOP1_MARGIN


def add_verdicts(device_result: dict) -> bool:
    all_pass = True
    for name, fp32 in device_result["fp32"]["prompts"].items():
        verdict = {
            "fp32_sdpa": fp32_comparison_passes(fp32["sdpa_vs_reference"]),
            "fp32_eager": fp32_comparison_passes(fp32["eager_vs_reference"]),
            "kv_cache": fp32["cache_vs_full_pass"]["max_abs_logit_diff"] <= CACHE_MAX_ABS_LOGIT_DIFF_LIMIT,
            "generation": fp32["generation_identical"],
            "bf16_drift": bf16_drift_passes(device_result["bf16"]["prompts"][name]),
        }
        device_result.setdefault("verdicts", {})[name] = verdict
        all_pass &= all(verdict.values())
    device_result["pass"] = all_pass
    return all_pass


def format_number(value: float) -> str:
    return f"{value:.2e}" if value < 0.01 else f"{value:.4f}"


def build_markdown(results: dict) -> str:
    lines = [
        "# Experiment 01: in-house Gemma 3 1B vs Hugging Face",
        "",
        f"**Overall: {'PASS' if results['pass'] else 'FAIL'}**",
        "",
        f"Model: `{MODEL_REPOSITORY}`. Reference: transformers {results['reference_version']} (eager attention).",
        "",
        "Pass criteria (fixed before the first run):",
        f"- fp32: max |Δlogit| ≤ {FP32_MAX_ABS_LOGIT_DIFF_LIMIT}, mean KL ≤ {FP32_MEAN_KL_LIMIT}, top-1 agreement 100%.",
        f"- bf16: our drift from fp32 ≤ {BF16_DRIFT_RATIO_LIMIT}× the reference's own bf16 drift; top-1 within {BF16_TOP1_MARGIN:.0%}.",
        f"- KV cache: decoding token by token matches one full pass, max |Δlogit| ≤ {CACHE_MAX_ABS_LOGIT_DIFF_LIMIT}.",
        f"- Generation: {GENERATED_TOKEN_COUNT} greedy tokens in fp32 are identical.",
        "",
    ]
    for device_name, device_result in results["devices"].items():
        lines += [f"## {device_name}: {'PASS' if device_result['pass'] else 'FAIL'}", ""]
        lines += [
            f"Load time, fp32: ours {device_result['fp32']['rastai_load_seconds']:.1f}s, "
            f"reference {device_result['fp32']['reference_load_seconds']:.1f}s.",
            "",
            "| Prompt | Tokens | fp32 SDPA max Δ | fp32 eager max Δ | fp32 KL | KV cache max Δ | Generation | bf16 drift ratio | bf16 top-1 (ours / ref) |",
            "|---|---|---|---|---|---|---|---|---|",
        ]
        for name, fp32 in device_result["fp32"]["prompts"].items():
            bf16 = device_result["bf16"]["prompts"][name]
            lines.append(
                f"| {name} | {fp32['token_count']} "
                f"| {format_number(fp32['sdpa_vs_reference']['max_abs_logit_diff'])} "
                f"| {format_number(fp32['eager_vs_reference']['max_abs_logit_diff'])} "
                f"| {format_number(fp32['sdpa_vs_reference']['mean_kl'])} "
                f"| {format_number(fp32['cache_vs_full_pass']['max_abs_logit_diff'])} "
                f"| {'identical' if fp32['generation_identical'] else 'DIFFERS'} ({fp32['generated_token_count']} tok) "
                f"| {bf16['drift_ratio']:.3f} "
                f"| {bf16['rastai_bf16_drift']['top1_agreement']:.1%} / {bf16['reference_bf16_drift']['top1_agreement']:.1%} |"
            )
        failed = {name: [check for check, ok in verdict.items() if not ok] for name, verdict in device_result["verdicts"].items()}
        failed = {name: checks for name, checks in failed.items() if checks}
        lines += ["", f"Failed checks: {failed if failed else 'none'}", ""]

    lines += ["## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    import transformers

    torch.set_float32_matmul_precision("highest")  # no TF32: fp32 must mean fp32 for parity
    model_folder = snapshot_path(MODEL_REPOSITORY)
    prompts = build_prompts(load_tokenizer(model_folder))
    stop_token_ids = load_stop_token_ids(model_folder)

    results = {
        "model": MODEL_REPOSITORY,
        "reference_version": transformers.__version__,
        "environment": environment_fingerprint(),
        "devices": {},
    }
    for device in DEVICES:
        label = device_label(device)
        print(f"[{EXPERIMENT_NAME}] {label}: fp32 checks", flush=True)
        fp32_results, fp32_reference = run_fp32_checks(model_folder, device, prompts, stop_token_ids)
        print(f"[{EXPERIMENT_NAME}] {label}: bf16 checks", flush=True)
        bf16_results = run_bf16_checks(model_folder, device, prompts, fp32_reference)
        results["devices"][label] = {"fp32": fp32_results, "bf16": bf16_results}
        add_verdicts(results["devices"][label])
        del fp32_reference
        gc.collect()

    results["pass"] = all(device_result["pass"] for device_result in results["devices"].values())
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, build_markdown(results))
    print(build_markdown(results))
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
