"""The extension-point base classes engines define and plugins implement (contracts 12.3; layout
frontend-and-plugins.md 11.2).

Each is a plain base class with a few methods. The methods every implementation must write raise
NotImplementedError; the optional ones do nothing by default, so a stateless recipe or a metric without setup
writes only what it needs. Recipe and Metric come first (M1); the other base classes of 11.2 join in the build step
that first uses them (11.6).
"""

from rastai_plugin.types import Batch, EvalCase, EvalContext, MetricResult, ModelOutput, StepResult, TrainingContext


class Recipe:
    """A training method. The trainer calls prepare, then training_step per batch. resume_state and restore_state
    carry the recipe's own state through a checkpoint."""

    name: str = ""

    def prepare(self, context: TrainingContext) -> None:
        """Called once before the first step: build reference models, extra loaders, anything per run."""

    def training_step(self, batch: Batch) -> StepResult:
        """The loss and metrics for one batch. The loop does backward and the optimizer step."""
        raise NotImplementedError(f"{type(self).__name__} must implement training_step")

    def resume_state(self) -> dict:
        """The recipe's own state, saved as recipe_state in resume.json. A stateless recipe saves nothing."""
        return {}

    def restore_state(self, state: dict) -> None:
        """Called once, after prepare, when a run resumes, with what resume_state returned."""


class Metric:
    """One evaluation metric. Judges, Pass@k, perplexity and drift are all Metrics."""

    # "<plugin_id>.<name>", e.g. "eval.pass_at_k": the manifest's metric_id.
    name: str = ""

    def prepare(self, context: EvalContext) -> None:
        """Called once per suite run: load a judge model, start a sandbox, read reference data."""

    def score(self, case: EvalCase, output: ModelOutput) -> MetricResult:
        """The verdict on one case."""
        raise NotImplementedError(f"{type(self).__name__} must implement score")
