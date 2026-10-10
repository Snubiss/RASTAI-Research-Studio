"""
Experiment 03: "Teach it", an in-house LoRA in under 3 minutes while the chat model stays loaded
(spec v5 §32, spike item 4).

On the RTX 5080 held to the First Run's 8 GiB budget:
  1. The chat model is loaded and answers held-out questions about Rastai (it can't know them) and
     general-knowledge questions. These are the "before" answers.
  2. "Teach it" is pressed. The clock starts: a training copy of the model is loaded, LoRA adapters are
     attached to every projection of every block, the 40 Q&A pairs are trained on, and the adapter is
     saved with the in-house safetensors writer. After every epoch the chat model answers a question,
     to show it keeps serving.
  3. The trained model answers the same questions. These are the "after" answers.

Pass criteria (fixed before the first run): Teach it finishes within 3 minutes, peak VRAM stays inside
8 GiB, at least 70% of held-out questions are answered correctly afterwards, and no general-knowledge
answer that was right before is wrong afterwards.

Run:  uv run python experiments/03_teach_it_lora.py
"""

import json
import time
from pathlib import Path

import torch

from rastai_spike.evaluation import accuracy, answer, ask_all
from rastai_spike.generation import generate_greedy, load_stop_token_ids
from rastai_spike.gemma3 import load_model
from rastai_spike.hf_cache import snapshot_path
from rastai_spike.lora import adapter_tensors, attach_lora, set_lora_enabled, trainable_parameters
from rastai_spike.prompts import encode, load_tokenizer
from rastai_spike.reporting import device_label, environment_fingerprint, write_results
from rastai_spike.safetensors_reader import SafetensorsFile
from rastai_spike.safetensors_writer import write_safetensors
from rastai_spike.training import TrainingSettings, chat_prompt, tokenize_example, train

EXPERIMENT_NAME = "03_teach_it_lora"
MODEL_REPOSITORY = "unsloth/gemma-3-1b-it"
DEVICE = "cuda:0"
MEMORY_CAP_GIB = 8.0
SPIKE_FOLDER = Path(__file__).resolve().parents[1]
DATA_FILE = SPIKE_FOLDER / "data" / "teach_it_rastai.json"
ARTIFACTS_FOLDER = SPIKE_FOLDER / "artifacts"
ADAPTER_FILE = ARTIFACTS_FOLDER / "teach_it_rastai_lora.safetensors"

LORA_RANK = 16
LORA_ALPHA = 32
SETTINGS = TrainingSettings(epochs=8, batch_size=8, learning_rate=3e-4, weight_decay=0.0, seed=0)
CHAT_CHECK_QUESTION = "Explain in two sentences why the sky is blue."
CHAT_CHECK_TOKENS = 32
GIB = 1024**3

# Pass criteria (fixed before the first run)
TEACH_SECONDS_LIMIT = 180
HELD_OUT_ACCURACY_REQUIRED = 0.7
GENERAL_REGRESSIONS_ALLOWED = 0


# ----------------------------------------------------------------------------------------------
# Teach it
# ----------------------------------------------------------------------------------------------

def chat_check(chat_model, tokenizer, stop_token_ids) -> dict:
    """The chat model answers while training is in progress (between epochs)."""
    prompt_ids = encode(tokenizer, chat_prompt(CHAT_CHECK_QUESTION))
    torch.cuda.synchronize(DEVICE)
    start = time.perf_counter()
    generated = generate_greedy(chat_model, prompt_ids, CHAT_CHECK_TOKENS, stop_token_ids)
    torch.cuda.synchronize(DEVICE)
    elapsed = time.perf_counter() - start
    return {"tokens": len(generated), "tokens_per_second": len(generated) / elapsed, "text": tokenizer.decode(generated, skip_special_tokens=True)}


def teach_it(model_folder, tokenizer, training_data, chat_model, stop_token_ids) -> dict:
    """Everything that happens after the user presses Teach it, timed end to end."""
    start = time.perf_counter()
    trainee = load_model(model_folder, DEVICE, torch.bfloat16)
    torch.manual_seed(SETTINGS.seed)  # LoRA's random initialisation
    adapters = attach_lora(trainee, LORA_RANK, LORA_ALPHA)
    examples = [tokenize_example(tokenizer, pair["question"], pair["answer"]) for pair in training_data]

    epoch_log = []

    def after_epoch(epoch, mean_loss):
        epoch_log.append({"epoch": epoch, "mean_loss": mean_loss, "chat_check": chat_check(chat_model, tokenizer, stop_token_ids)})
        print(f"[{EXPERIMENT_NAME}] epoch {epoch}: loss {mean_loss:.3f}", flush=True)

    history = train(trainee, trainable_parameters(adapters), examples, SETTINGS, after_epoch)

    ARTIFACTS_FOLDER.mkdir(exist_ok=True)
    metadata = {"base_model": MODEL_REPOSITORY, "rank": str(LORA_RANK), "alpha": str(LORA_ALPHA), "format": "rastai.lora/0"}
    write_safetensors(ADAPTER_FILE, adapter_tensors(adapters), metadata)
    teach_seconds = time.perf_counter() - start

    return {
        "trainee": trainee,
        "adapters": adapters,
        "teach_seconds": teach_seconds,
        "training_seconds": sum(step["step_seconds"] for step in history),
        "steps": len(history),
        "examples": len(examples),
        "answer_tokens": sum(sum(label != -100 for label in example.labels) for example in examples),
        "trainable_parameters": sum(parameter.numel() for parameter in trainable_parameters(adapters)),
        "epochs": epoch_log,
        "history": history,
    }


def adapter_round_trips(adapters) -> bool:
    """The saved file, read back with our own reader, must hold exactly the trained weights."""
    expected = adapter_tensors(adapters)
    with SafetensorsFile(ADAPTER_FILE) as saved:
        return set(saved.names()) == set(expected) and all(
            torch.equal(saved.load(name), tensor.cpu()) for name, tensor in expected.items()
        )


# ----------------------------------------------------------------------------------------------
# Report
# ----------------------------------------------------------------------------------------------

def build_markdown(results: dict) -> str:
    teach = results["teach_it"]
    verdicts = results["verdicts"]
    lines = [
        "# Experiment 03: Teach it (in-house LoRA while the chat model stays loaded)",
        "",
        f"**Overall: {'PASS' if all(verdicts.values()) else 'FAIL'}**  ({', '.join(f'{name}: {ok}' for name, ok in verdicts.items())})",
        "",
        f"Device: {results['device']}, capped at {MEMORY_CAP_GIB:g} GiB. Chat model and training copy both resident.",
        "",
        "| Measure | Value |",
        "|---|---|",
        f"| Teach it, end to end (load copy, attach LoRA, tokenize, train, save) | **{teach['teach_seconds']:.1f} s** (limit {TEACH_SECONDS_LIMIT} s) |",
        f"| Of which training steps | {teach['training_seconds']:.1f} s for {teach['steps']} steps |",
        f"| Training data | {teach['examples']} Q&A pairs, {teach['answer_tokens']} answer tokens, {SETTINGS.epochs} epochs |",
        f"| LoRA | rank {LORA_RANK}, alpha {LORA_ALPHA}, all 7 projections × 26 layers, {teach['trainable_parameters']:,} trainable parameters |",
        f"| Peak VRAM (both models + training) | {results['peak_memory_gib']:.2f} GiB |",
        f"| Adapter file | {results['adapter_megabytes']:.1f} MB, round-trips through our reader: {results['adapter_round_trips']} |",
        f"| Held-out accuracy, before → after | {results['held_out_before']:.0%} → **{results['held_out_after']:.0%}** |",
        f"| General accuracy, before → after | {results['general_before']:.0%} → {results['general_after']:.0%} |",
        f"| Adapter switched off reproduces the base model | {results['adapter_off_matches_base']} |",
        "",
        "Loss and chat speed per epoch (the chat model answering between epochs):",
        "",
        "| Epoch | Mean loss | Chat tokens/s |",
        "|---|---|---|",
        *[f"| {epoch['epoch']} | {epoch['mean_loss']:.3f} | {epoch['chat_check']['tokens_per_second']:.1f} |" for epoch in teach["epochs"]],
        "",
        "## Held-out questions (new phrasings, never trained on)",
        "",
    ]
    for before, after in zip(results["held_out"]["before"], results["held_out"]["after"]):
        lines += [
            f"**{before['question']}**",
            f"- Before ({'✓' if before['correct'] else '✗'}): {before['answer']}",
            f"- After ({'✓' if after['correct'] else '✗'}): {after['answer']}",
            "",
        ]
    lines += ["## General knowledge (must not regress)", ""]
    for before, after in zip(results["general"]["before"], results["general"]["after"]):
        lines.append(f"- {before['question']} — before {'✓' if before['correct'] else '✗'}, after {'✓' if after['correct'] else '✗'}: {after['answer'][:120]}")
    lines += ["", "## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", ""]
    return "\n".join(lines)


def main() -> None:
    torch.cuda.set_device(DEVICE)
    total_memory = torch.cuda.get_device_properties(DEVICE).total_memory
    torch.cuda.set_per_process_memory_fraction(MEMORY_CAP_GIB * GIB / total_memory, DEVICE)
    torch.cuda.reset_peak_memory_stats(DEVICE)

    model_folder = snapshot_path(MODEL_REPOSITORY)
    tokenizer = load_tokenizer(model_folder)
    stop_token_ids = load_stop_token_ids(model_folder)
    data = json.loads(DATA_FILE.read_text())

    chat_model = load_model(model_folder, DEVICE, torch.bfloat16)
    held_out_before = ask_all(chat_model, tokenizer, stop_token_ids, data["held_out"])
    general_before = ask_all(chat_model, tokenizer, stop_token_ids, data["general"])

    teach = teach_it(model_folder, tokenizer, data["train"], chat_model, stop_token_ids)
    trainee, adapters = teach.pop("trainee"), teach.pop("adapters")

    held_out_after = ask_all(trainee, tokenizer, stop_token_ids, data["held_out"])
    general_after = ask_all(trainee, tokenizer, stop_token_ids, data["general"])

    set_lora_enabled(adapters, False)
    adapter_off_answer = answer(trainee, tokenizer, stop_token_ids, data["general"][0]["question"])
    set_lora_enabled(adapters, True)

    general_regressions = sum(before["correct"] and not after["correct"] for before, after in zip(general_before, general_after))
    results = {
        "device": device_label(DEVICE),
        "teach_it": teach,
        "peak_memory_gib": torch.cuda.max_memory_allocated(DEVICE) / GIB,
        "adapter_megabytes": ADAPTER_FILE.stat().st_size / 1e6,
        "adapter_round_trips": adapter_round_trips(adapters),
        "adapter_off_matches_base": adapter_off_answer == general_before[0]["answer"],
        "held_out": {"before": held_out_before, "after": held_out_after},
        "general": {"before": general_before, "after": general_after},
        "held_out_before": accuracy(held_out_before),
        "held_out_after": accuracy(held_out_after),
        "general_before": accuracy(general_before),
        "general_after": accuracy(general_after),
        "environment": environment_fingerprint(),
    }
    results["verdicts"] = {
        "time": teach["teach_seconds"] <= TEACH_SECONDS_LIMIT,
        "memory": results["peak_memory_gib"] <= MEMORY_CAP_GIB,
        "held_out": results["held_out_after"] >= HELD_OUT_ACCURACY_REQUIRED,
        "no_regressions": general_regressions <= GENERAL_REGRESSIONS_ALLOWED,
    }

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
