"""
Behaviour checks for the Teach it experiments (an early form of spec v5 §10's behaviour tests and §9's model diff).

Two kinds of check:
  - keyword answers: ask a question, greedy-decode, and look for an expected keyword;
  - drift: how far a trained model's next-token distributions moved from the base model's on prompts it was
    never trained on, measured as KL(base ‖ trained) along the base model's own answers. Zero means unchanged.
"""

import torch
import torch.nn.functional as F

from rastai_spike.generation import generate_greedy
from rastai_spike.prompts import encode
from rastai_spike.training import chat_prompt

ANSWER_TOKEN_LIMIT = 48


def answer_ids(model, tokenizer, stop_token_ids, question: str, token_limit: int = ANSWER_TOKEN_LIMIT) -> list[int]:
    return generate_greedy(model, encode(tokenizer, chat_prompt(question)), token_limit, stop_token_ids)


def answer(model, tokenizer, stop_token_ids, question: str, token_limit: int = ANSWER_TOKEN_LIMIT) -> str:
    generated = answer_ids(model, tokenizer, stop_token_ids, question, token_limit)
    return tokenizer.decode(generated, skip_special_tokens=True).strip()


def is_correct(answer_text: str, keywords: list[str]) -> bool:
    lowered = answer_text.lower()
    return any(keyword.lower() in lowered for keyword in keywords)


def ask_all(model, tokenizer, stop_token_ids, items: list[dict]) -> list[dict]:
    results = []
    for item in items:
        text = answer(model, tokenizer, stop_token_ids, item["question"])
        results.append({"question": item["question"], "answer": text, "correct": is_correct(text, item["keywords"])})
    return results


def accuracy(results: list[dict]) -> float:
    return sum(result["correct"] for result in results) / len(results)


@torch.inference_mode()
def answer_drift(base_model, trained_model, tokenizer, question: str, base_answer_ids: list[int]) -> float:
    """Mean KL(base ‖ trained) over the positions that predict the base model's answer tokens."""
    prompt_ids = encode(tokenizer, chat_prompt(question))
    device = base_model.token_embedding.weight.device
    sequence = torch.tensor([prompt_ids + base_answer_ids], device=device)

    first_prediction = len(prompt_ids) - 1  # the position whose output predicts the first answer token
    last_prediction = sequence.shape[1] - 1  # exclusive: the final token predicts nothing we scored
    base_log_probs = F.log_softmax(base_model(sequence)[0, first_prediction:last_prediction].float(), dim=-1)
    trained_log_probs = F.log_softmax(trained_model(sequence)[0, first_prediction:last_prediction].float(), dim=-1)
    kl_per_position = (base_log_probs.exp() * (base_log_probs - trained_log_probs)).sum(-1)
    return kl_per_position.mean().item()
