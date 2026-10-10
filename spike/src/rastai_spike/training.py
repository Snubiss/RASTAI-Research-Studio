"""
In-house supervised fine-tuning (SFT) on the unified runtime (spec v5 §10 and §17).

Each example is a chat turn: the user's question, then the model's answer. The loss counts only the answer
tokens (and the end-of-turn token, so the model learns where to stop); the question is context, not a target.
Sequences in a batch are padded on the right. Causal attention means padding never influences real tokens,
so no padding mask is needed, only a loss mask.
"""

import random
import time
from dataclasses import dataclass
from typing import Callable

import torch
import torch.nn.functional as F
from tokenizers import Tokenizer

from rastai_spike.gemma3 import Gemma3Model
from rastai_spike.prompts import encode

IGNORED_LABEL = -100
PAD_TOKEN_ID = 0
USER_TURN = "<start_of_turn>user\n{question}<end_of_turn>\n<start_of_turn>model\n"
MODEL_ANSWER = "{answer}<end_of_turn>\n"


@dataclass(frozen=True)
class TrainingSettings:
    epochs: int
    batch_size: int
    learning_rate: float
    weight_decay: float
    seed: int
    gradient_checkpointing: bool = True  # trades ~30% more compute for most of the activation memory


@dataclass
class TokenizedExample:
    token_ids: list[int]
    labels: list[int]  # IGNORED_LABEL where the token is context, not a target


def chat_prompt(question: str) -> str:
    return USER_TURN.format(question=question)


def tokenize_example(tokenizer: Tokenizer, question: str, answer: str) -> TokenizedExample:
    prompt_ids = encode(tokenizer, chat_prompt(question))
    answer_ids = tokenizer.encode(MODEL_ANSWER.format(answer=answer), add_special_tokens=False).ids
    return TokenizedExample(
        token_ids=prompt_ids + answer_ids,
        labels=[IGNORED_LABEL] * len(prompt_ids) + answer_ids,
    )


def make_batches(examples: list[TokenizedExample], batch_size: int, shuffler: random.Random, device):
    """Shuffle, group and right-pad. Returns (token_ids, labels) tensor pairs."""
    order = list(range(len(examples)))
    shuffler.shuffle(order)
    batches = []
    for start in range(0, len(order), batch_size):
        group = [examples[index] for index in order[start : start + batch_size]]
        longest = max(len(example.token_ids) for example in group)
        token_ids = [example.token_ids + [PAD_TOKEN_ID] * (longest - len(example.token_ids)) for example in group]
        labels = [example.labels + [IGNORED_LABEL] * (longest - len(example.labels)) for example in group]
        batches.append((torch.tensor(token_ids, device=device), torch.tensor(labels, device=device)))
    return batches


def next_token_loss(model: Gemma3Model, token_ids: torch.Tensor, labels: torch.Tensor) -> torch.Tensor:
    """Position t predicts token t+1. Only positions that predict an answer token are unembedded:
    with a 262k vocabulary, logits for the prompt and padding would cost gigabytes and add nothing."""
    hidden = model(token_ids, unembed_output=False)
    targets = labels[:, 1:]
    scored = targets != IGNORED_LABEL
    logits = model.unembed(hidden[:, :-1][scored])  # [scored positions, vocabulary]
    return F.cross_entropy(logits.float(), targets[scored])


def train(
    model: Gemma3Model,
    parameters: list[torch.nn.Parameter],
    examples: list[TokenizedExample],
    settings: TrainingSettings,
    after_epoch: Callable[[int, float], None] | None = None,
) -> list[dict]:
    """Run the SFT loop. Returns one record per optimizer step."""
    device = model.token_embedding.weight.device
    optimizer = torch.optim.AdamW(parameters, lr=settings.learning_rate, weight_decay=settings.weight_decay)
    shuffler = random.Random(settings.seed)
    model.gradient_checkpointing = settings.gradient_checkpointing
    model.train()

    history = []
    for epoch in range(settings.epochs):
        epoch_losses = []
        for token_ids, labels in make_batches(examples, settings.batch_size, shuffler, device):
            step_start = time.perf_counter()
            loss = next_token_loss(model, token_ids, labels)
            optimizer.zero_grad(set_to_none=True)
            loss.backward()
            optimizer.step()

            loss_value = loss.item()  # also waits for the GPU, so the step time below is real
            epoch_losses.append(loss_value)
            history.append({"epoch": epoch, "loss": loss_value, "step_seconds": time.perf_counter() - step_start})

        if after_epoch is not None:
            after_epoch(epoch, sum(epoch_losses) / len(epoch_losses))

    model.gradient_checkpointing = False
    model.eval()
    return history
