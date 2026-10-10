"""
Experiment 03b: can Teach it learn new facts without changing everything else? (follow-up to experiment 03)

Experiment 03 passed its keyword checks, but reading the answers showed side effects: the trained model
adopted the training data's terse style, and invented a "Pedro de Alva" editor for Romeo and Juliet.
Keyword checks can't see that, so this experiment measures drift directly and tries the standard fix.

Recipes compared (all rank-16 LoRA, same seed, same learning rate):
  A. Rastai facts only, 8 epochs (experiment 03's recipe).
  B. Rastai facts + anchors, 8 epochs. Anchors are the base model's OWN answers to 40 unrelated questions,
     so training is also asked to keep answering those the way it already did.
  C. Rastai facts + anchors, 4 epochs.

Measures: held-out accuracy on new phrasings of the facts; drift = mean KL(base ‖ trained) on 20 unrelated
prompts never trained on; how many of those 20 greedy answers come out word-for-word the same as before.

Success (fixed before the first run): an anchored recipe keeps held-out accuracy ≥ 70% while cutting drift
by at least half compared with recipe A.

Run:  uv run python experiments/03b_teach_it_drift.py
"""

import gc
import json
import time
from pathlib import Path

import torch

from rastai_spike.evaluation import accuracy, answer, answer_drift, answer_ids, ask_all
from rastai_spike.generation import load_stop_token_ids
from rastai_spike.gemma3 import load_model
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.lora import attach_lora, trainable_parameters
from rastai_spike.prompts import load_tokenizer
from rastai_spike.reporting import device_label, environment_fingerprint, write_results
from rastai_spike.training import TrainingSettings, tokenize_example, train

EXPERIMENT_NAME = "03b_teach_it_drift"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
DEVICE = "cuda:0"
MEMORY_CAP_GIB = 8.0
SPIKE_FOLDER = Path(__file__).resolve().parents[1]
TEACH_DATA_FILE = SPIKE_FOLDER / "data" / "teach_it_rastai.json"
GENERAL_PROMPTS_FILE = SPIKE_FOLDER / "data" / "general_prompts.json"

LORA_RANK = 16
LORA_ALPHA = 32
LEARNING_RATE = 3e-4
BATCH_SIZE = 8
SEED = 0
ANCHOR_ANSWER_TOKENS = 96  # long enough to carry the base model's style, short enough to stay cheap
DRIFT_ANSWER_TOKENS = 64
WATCHED_QUESTION = "Who wrote Romeo and Juliet?"  # where experiment 03's recipe invented a fact
GIB = 1024**3

RECIPES = [  # (name, use anchors, epochs)
    ("A: facts only, 8 epochs", False, 8),
    ("B: facts + anchors, 8 epochs", True, 8),
    ("C: facts + anchors, 4 epochs", True, 4),
]

# Success criteria (fixed before the first run)
HELD_OUT_ACCURACY_REQUIRED = 0.7
DRIFT_REDUCTION_REQUIRED = 0.5


# ----------------------------------------------------------------------------------------------
# Base-model reference answers
# ----------------------------------------------------------------------------------------------

def base_answers(base_model, tokenizer, stop_token_ids, questions: list[str], token_limit: int) -> dict[str, list[int]]:
    return {question: answer_ids(base_model, tokenizer, stop_token_ids, question, token_limit) for question in questions}


def anchor_pairs(tokenizer, anchors: dict[str, list[int]]) -> list[dict]:
    """The base model's own answers become training targets, so training holds them in place."""
    return [{"question": question, "answer": tokenizer.decode(ids, skip_special_tokens=True).strip()} for question, ids in anchors.items()]


# ----------------------------------------------------------------------------------------------
# One recipe
# ----------------------------------------------------------------------------------------------

def run_recipe(name, use_anchors, epochs, context) -> dict:
    print(f"[{EXPERIMENT_NAME}] {name}", flush=True)
    tokenizer, stop_token_ids, base_model = context["tokenizer"], context["stop_token_ids"], context["base_model"]
    training_pairs = context["teach_data"]["train"] + (context["anchor_pairs"] if use_anchors else [])

    start = time.perf_counter()
    trainee = load_model(context["model_folder"], DEVICE, torch.bfloat16)
    torch.manual_seed(SEED)
    adapters = attach_lora(trainee, LORA_RANK, LORA_ALPHA)
    examples = [tokenize_example(tokenizer, pair["question"], pair["answer"]) for pair in training_pairs]
    settings = TrainingSettings(epochs=epochs, batch_size=BATCH_SIZE, learning_rate=LEARNING_RATE, weight_decay=0.0, seed=SEED)
    history = train(trainee, trainable_parameters(adapters), examples, settings)
    teach_seconds = time.perf_counter() - start

    drift_answers = context["drift_base_answers"]
    drift_per_prompt = [answer_drift(base_model, trainee, tokenizer, question, ids) for question, ids in drift_answers.items()]
    unchanged_answers = sum(
        answer_ids(trainee, tokenizer, stop_token_ids, question, DRIFT_ANSWER_TOKENS) == ids
        for question, ids in drift_answers.items()
    )
    held_out = ask_all(trainee, tokenizer, stop_token_ids, context["teach_data"]["held_out"])
    general = ask_all(trainee, tokenizer, stop_token_ids, context["teach_data"]["general"])

    result = {
        "teach_seconds": teach_seconds,
        "steps": len(history),
        "final_epoch_loss": sum(step["loss"] for step in history if step["epoch"] == epochs - 1) / max(1, sum(step["epoch"] == epochs - 1 for step in history)),
        "held_out_accuracy": accuracy(held_out),
        "general_accuracy": accuracy(general),
        "mean_drift_kl": sum(drift_per_prompt) / len(drift_per_prompt),
        "max_drift_kl": max(drift_per_prompt),
        "unchanged_answers": unchanged_answers,
        "watched_answer": answer(trainee, tokenizer, stop_token_ids, WATCHED_QUESTION),
        "held_out": held_out,
    }
    del trainee, adapters
    gc.collect()
    torch.cuda.empty_cache()
    return result


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def add_verdict(results: dict) -> None:
    recipes = results["recipes"]
    baseline_drift = recipes[RECIPES[0][0]]["mean_drift_kl"]
    for name, use_anchors, _ in RECIPES:
        recipe = recipes[name]
        recipe["drift_vs_A"] = recipe["mean_drift_kl"] / baseline_drift
    results["success"] = any(
        recipes[name]["held_out_accuracy"] >= HELD_OUT_ACCURACY_REQUIRED and recipes[name]["drift_vs_A"] <= 1 - DRIFT_REDUCTION_REQUIRED
        for name, use_anchors, _ in RECIPES if use_anchors
    )


def build_markdown(results: dict) -> str:
    drift_count = len(results["drift_prompts"])
    lines = [
        "# Experiment 03b: Teach it without side effects",
        "",
        f"**Success: {results['success']}** (an anchored recipe keeps held-out ≥ {HELD_OUT_ACCURACY_REQUIRED:.0%} and cuts drift by ≥ {DRIFT_REDUCTION_REQUIRED:.0%} vs A)",
        "",
        f"Device: {results['device']}, capped at {MEMORY_CAP_GIB:g} GiB. Anchor answers from the base model took {results['anchor_seconds']:.1f} s "
        f"(once per base model; cacheable, so not counted in Teach it time).",
        "",
        f"| Recipe | Teach it time | Steps | Last-epoch loss | Held-out accuracy | General keywords | Mean drift KL | Drift vs A | Unchanged answers (of {drift_count}) |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for name, recipe in results["recipes"].items():
        lines.append(
            f"| {name} | {recipe['teach_seconds']:.1f} s | {recipe['steps']} | {recipe['final_epoch_loss']:.3f} | "
            f"{recipe['held_out_accuracy']:.0%} | {recipe['general_accuracy']:.0%} | {recipe['mean_drift_kl']:.4f} | "
            f"{recipe['drift_vs_A']:.0%} | {recipe['unchanged_answers']} |"
        )
    lines += ["", f"**{WATCHED_QUESTION}** (experiment 03's recipe invented an editor here)", "",
              f"- Base model: {results['watched_base_answer']}"]
    lines += [f"- {name}: {recipe['watched_answer']}" for name, recipe in results["recipes"].items()]
    lines += ["", "Held-out answers per recipe:", ""]
    for index, item in enumerate(results["held_out_questions"]):
        lines.append(f"**{item}**")
        for name, recipe in results["recipes"].items():
            answer_record = recipe["held_out"][index]
            lines.append(f"- {name.split(':')[0]} ({'✓' if answer_record['correct'] else '✗'}): {answer_record['answer'][:160]}")
        lines.append("")
    lines += ["## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    torch.cuda.set_device(DEVICE)
    total_memory = torch.cuda.get_device_properties(DEVICE).total_memory
    torch.cuda.set_per_process_memory_fraction(MEMORY_CAP_GIB * GIB / total_memory, DEVICE)

    model_folder = snapshot_path(MODEL_REPOSITORY)
    tokenizer = load_tokenizer(model_folder)
    stop_token_ids = load_stop_token_ids(model_folder)
    teach_data = json.loads(TEACH_DATA_FILE.read_text())
    general_prompts = json.loads(GENERAL_PROMPTS_FILE.read_text())
    base_model = load_model(model_folder, DEVICE, torch.bfloat16)

    anchor_start = time.perf_counter()
    anchors = base_answers(base_model, tokenizer, stop_token_ids, general_prompts["anchor"], ANCHOR_ANSWER_TOKENS)
    anchor_seconds = time.perf_counter() - anchor_start

    context = {
        "model_folder": model_folder,
        "tokenizer": tokenizer,
        "stop_token_ids": stop_token_ids,
        "base_model": base_model,
        "teach_data": teach_data,
        "anchor_pairs": anchor_pairs(tokenizer, anchors),
        "drift_base_answers": base_answers(base_model, tokenizer, stop_token_ids, general_prompts["drift_eval"], DRIFT_ANSWER_TOKENS),
    }
    results = {
        "device": device_label(DEVICE),
        "anchor_seconds": anchor_seconds,
        "drift_prompts": general_prompts["drift_eval"],
        "held_out_questions": [item["question"] for item in teach_data["held_out"]],
        "watched_base_answer": answer(base_model, tokenizer, stop_token_ids, WATCHED_QUESTION),
        "recipes": {name: run_recipe(name, use_anchors, epochs, context) for name, use_anchors, epochs in RECIPES},
        "peak_memory_gib": torch.cuda.max_memory_allocated(DEVICE) / GIB,
        "environment": environment_fingerprint(),
    }
    add_verdict(results)

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
