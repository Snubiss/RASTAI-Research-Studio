"""
Experiment 01b: why did one bf16 top-1 token differ on the RTX 5080 (chat prompt)?

Experiment 01 found 1 of 19 positions where our bf16 (SDPA) prediction differed from the fp32 reference,
while the reference's own bf16 run matched. Two possible explanations:
  - a bug, in which case our bf16 eager path would also disagree with the reference's bf16 eager path;
  - a near-tie, where the fp32 gap between the top two tokens is smaller than bf16 rounding noise,
    and the SDPA kernel's different summation order tips it.

Run:  uv run python experiments/01b_bf16_flip_investigation.py
"""

import torch
from torch.nn.attention import SDPBackend, sdpa_kernel
from transformers import AutoModelForCausalLM

from rastai_spike.gemma3 import CAPTURE_FULL_ATTENTION, CAPTURE_NO_ATTENTION, load_model
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.prompts import build_prompts, load_tokenizer
from rastai_spike.reporting import device_label, environment_fingerprint, write_results

EXPERIMENT_NAME = "01b_bf16_flip_investigation"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
DEVICE = "cuda:0"
PROMPT_NAME = "chat"
SDPA_BACKENDS = {
    "default": None,
    "math": SDPBackend.MATH,
    "efficient": SDPBackend.EFFICIENT_ATTENTION,
    "cudnn": SDPBackend.CUDNN_ATTENTION,
}


@torch.inference_mode()
def reference_logits(model_folder, prompt_ids, dtype) -> torch.Tensor:
    model = AutoModelForCausalLM.from_pretrained(model_folder, dtype=dtype, attn_implementation="eager").to(DEVICE)
    logits = model(input_ids=torch.tensor([prompt_ids], device=DEVICE)).logits[0].float().cpu()
    del model
    torch.cuda.empty_cache()
    return logits


@torch.inference_mode()
def rastai_logits(model, prompt_ids, use_eager: bool, backend) -> torch.Tensor | None:
    capture = CAPTURE_FULL_ATTENTION if use_eager else CAPTURE_NO_ATTENTION
    input_ids = torch.tensor([prompt_ids], device=DEVICE)
    try:
        if backend is None:
            logits = model(input_ids, attention_capture=capture)
        else:
            with sdpa_kernel(backend):
                logits = model(input_ids, attention_capture=capture)
    except RuntimeError:
        return None  # this backend can't run this configuration on this GPU
    return logits[0].float().cpu()


def top_two_gap(logits: torch.Tensor) -> torch.Tensor:
    top_two = logits.topk(2, dim=-1).values
    return top_two[:, 0] - top_two[:, 1]


def disagreeing_positions(reference: torch.Tensor, candidate: torch.Tensor) -> list[int]:
    return (reference.argmax(-1) != candidate.argmax(-1)).nonzero().flatten().tolist()


def describe_position(tokenizer, prompt_ids, position, fp32_logits, candidates: dict) -> dict:
    top_two_ids = fp32_logits[position].topk(2).indices.tolist()
    return {
        "position": position,
        "context_ends_with": tokenizer.decode(prompt_ids[max(0, position - 6) : position + 1]),
        "fp32_top_two_tokens": [tokenizer.decode([token_id]) for token_id in top_two_ids],
        "fp32_top_two_gap": top_two_gap(fp32_logits)[position].item(),
        "chosen_token_by_run": {
            name: tokenizer.decode([logits[position].argmax().item()]) for name, logits in candidates.items()
        },
    }


def main() -> None:
    torch.set_float32_matmul_precision("highest")
    model_folder = snapshot_path(MODEL_REPOSITORY)
    tokenizer = load_tokenizer(model_folder)
    prompt_ids = build_prompts(tokenizer)[PROMPT_NAME]

    fp32_reference = reference_logits(model_folder, prompt_ids, torch.float32)
    bf16_reference = reference_logits(model_folder, prompt_ids, torch.bfloat16)

    ours = load_model(model_folder, DEVICE, torch.bfloat16)
    candidates = {"reference_bf16_eager": bf16_reference}
    candidates["rastai_bf16_eager"] = rastai_logits(ours, prompt_ids, use_eager=True, backend=None)
    for backend_name, backend in SDPA_BACKENDS.items():
        logits = rastai_logits(ours, prompt_ids, use_eager=False, backend=backend)
        if logits is not None:
            candidates[f"rastai_bf16_sdpa_{backend_name}"] = logits

    flipped = sorted({position for logits in candidates.values() for position in disagreeing_positions(fp32_reference, logits)})
    gaps = top_two_gap(fp32_reference)
    results = {
        "device": device_label(DEVICE),
        "prompt": PROMPT_NAME,
        "eager_bf16_bit_identical_to_reference": torch.equal(candidates["rastai_bf16_eager"], bf16_reference),
        "eager_bf16_max_abs_diff_vs_reference": (candidates["rastai_bf16_eager"] - bf16_reference).abs().max().item(),
        "disagreements_with_fp32_by_run": {
            name: disagreeing_positions(fp32_reference, logits) for name, logits in candidates.items()
        },
        "flipped_positions": [describe_position(tokenizer, prompt_ids, position, fp32_reference, candidates) for position in flipped],
        "fp32_median_top_two_gap": gaps.median().item(),
        "environment": environment_fingerprint(),
    }

    markdown = "\n".join([
        "# Experiment 01b: the single bf16 top-1 flip on the RTX 5080",
        "",
        f"- Our bf16 **eager** path bit-identical to the reference's bf16 eager path: "
        f"**{results['eager_bf16_bit_identical_to_reference']}** (max |Δ| {results['eager_bf16_max_abs_diff_vs_reference']:.3g}).",
        f"- Median fp32 gap between the top two tokens on this prompt: {results['fp32_median_top_two_gap']:.3f} logits.",
        "",
        "Positions where any bf16 run disagrees with fp32:",
        "",
        "| Run | Disagreeing positions |",
        "|---|---|",
        *[f"| {name} | {positions or 'none'} |" for name, positions in results["disagreements_with_fp32_by_run"].items()],
        "",
        *[
            f"- Position {item['position']} (context `{item['context_ends_with']!r}`): fp32 top two "
            f"{item['fp32_top_two_tokens']} separated by **{item['fp32_top_two_gap']:.4f}** logits; "
            f"chosen per run: {item['chosen_token_by_run']}"
            for item in results["flipped_positions"]
        ],
        "",
    ])
    write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)


if __name__ == "__main__":
    main()
