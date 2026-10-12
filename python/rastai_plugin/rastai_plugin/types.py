"""Plain dataclasses the extension points pass around (layout frontend-and-plugins.md 11.1).

Only types that exist nowhere else live here: document types come from rastai_formats and wire types from
rastai_proto, and neither is ever declared again. A type gets its fields with the task that first builds or reads
it, so the ones nothing fills yet are empty: `TrainingContext` and `Batch` grow with the trainer (M1-38), and
`EvalContext`, `EvalCase` and `ModelOutput` with the Evaluation plugin (M5).
"""

from dataclasses import dataclass, field
from typing import Any


@dataclass
class TrainingContext:
    """What a recipe's prepare() gets from the trainer: the model, the data, the optimizer and the run's settings.
    rastai_train builds it (build_training_context); its fields arrive with the trainer (M1-38)."""


@dataclass
class Batch:
    """One batch from the run's loader, handed to Recipe.training_step. Its fields arrive with the trainer (M1-38)."""


@dataclass(frozen=True)
class StepResult:
    """What Recipe.training_step returns. The trainer's loop owns backward, the optimizer and checkpoints, so a
    recipe only computes the loss (layout python.md 5.2)."""

    # A scalar torch tensor that still carries its graph: the loop calls backward() on it. Typed Any because only
    # NodeType, Lens and Sampler may name torch in this package (layout frontend-and-plugins.md 11.3).
    loss: Any
    # Recipe metrics by name, prefixed with the recipe ("dpo.reward_margin"); the loop adds the standard train.*
    # metrics itself.
    metrics: dict[str, float] = field(default_factory=dict)


@dataclass
class EvalContext:
    """What Metric.prepare gets once per suite run: the judge model, a sandbox, reference data. Its fields arrive
    with the Evaluation plugin (M5)."""


@dataclass(frozen=True)
class EvalCase:
    """One test of an eval suite, as a metric sees it. Its fields arrive with the Evaluation plugin (M5)."""


@dataclass(frozen=True)
class ModelOutput:
    """What the model under test answered for one case. Its fields arrive with the Evaluation plugin (M5)."""


@dataclass(frozen=True)
class MetricResult:
    """One metric's verdict on one case; every MetricResult is kept in eval.test_result and the result file
    (training-evaluation flow 9.1)."""

    score: float
    # The test's pass threshold when the runner compares against one; None when the score is only reported.
    threshold: float | None = None
    # Anything the metric's result view shows beside the score: matched keywords, judge reasoning, sample counts.
    details: dict = field(default_factory=dict)
