"""The extension-point base classes, the plain types and JobEvents (rastai_plugin/extension_points.py, types.py,
job_events.py): required methods raise, optional ones have safe defaults."""

import dataclasses

import pytest

from rastai_plugin import (
    Batch,
    EvalCase,
    EvalContext,
    JobEvents,
    Metric,
    MetricResult,
    ModelOutput,
    Recipe,
    StepResult,
    TrainingContext,
)


class ConstantMetric(Metric):
    name = "test.constant"

    def score(self, case: EvalCase, output: ModelOutput) -> MetricResult:
        return MetricResult(score=1.0)


def test_recipe_defaults_let_a_stateless_recipe_write_only_training_step() -> None:
    recipe = Recipe()
    recipe.prepare(TrainingContext())
    assert recipe.resume_state() == {}
    recipe.restore_state({})
    with pytest.raises(NotImplementedError, match="training_step"):
        recipe.training_step(Batch())


def test_metric_score_must_be_written_and_prepare_is_optional() -> None:
    with pytest.raises(NotImplementedError, match="score"):
        Metric().score(EvalCase(), ModelOutput())
    metric = ConstantMetric()
    metric.prepare(EvalContext())
    assert metric.score(EvalCase(), ModelOutput()) == MetricResult(score=1.0, threshold=None, details={})


def test_names_default_to_empty() -> None:
    assert Recipe.name == ""
    assert Metric.name == ""


def test_results_are_frozen_with_their_own_defaults() -> None:
    first = StepResult(loss=0.5)
    second = StepResult(loss=0.25)
    first.metrics["tiny.accuracy"] = 1.0
    assert second.metrics == {}
    with pytest.raises(dataclasses.FrozenInstanceError):
        first.loss = 0.0
    result = MetricResult(score=0.5)
    with pytest.raises(dataclasses.FrozenInstanceError):
        result.score = 1.0


def test_job_events_methods_must_be_written() -> None:
    events = JobEvents()
    with pytest.raises(NotImplementedError, match="emit_durable"):
        events.emit_durable("run.metrics", {"step": 1})
    with pytest.raises(NotImplementedError, match="emit_ephemeral"):
        events.emit_ephemeral("job.progress", {"fraction": 0.5})
