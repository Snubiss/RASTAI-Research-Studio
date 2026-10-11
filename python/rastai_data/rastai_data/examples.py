"""The training examples rastai_train and rastai_rl consume (layout python.md section 4.2).

ImageExample joins this file in §31 step 6, with the image decoder it needs (layout section 4.8).
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class TrainingExample:
    token_ids: list[int]
    # True where the token is a target: answer tokens and the end-of-turn token.
    target_mask: list[bool]


@dataclass(frozen=True)
class PreferencePair:
    prompt_ids: list[int]
    chosen_ids: list[int]
    rejected_ids: list[int]


@dataclass(frozen=True)
class LabeledExample:
    token_ids: list[int]
    label_index: int


@dataclass(frozen=True)
class PromptExample:
    # For RL rollouts. The gateway applies the chat template, so these stay as messages, not token ids.
    messages: list[dict]
    # The task check, the expected answer, or an empty dict.
    reference: dict
