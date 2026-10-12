"""rastai_plugin: the Python plugin SDK (contracts 2.2; layout frontend-and-plugins.md 11).

The low layer engines define their extension points on: the registry, the extension-point base classes and the
plain types they pass, the job-side types every job uses (WorkerHost, JobEvents, JobControl), the module loader that
checks a plugin's register() against its manifest, and the generated error and term catalog (rastai_plugin.catalog).
"""

from rastai_plugin.errors import PluginError
from rastai_plugin.extension_points import Metric, Recipe
from rastai_plugin.host import PluginHost, WorkerHost, connect_worker_host
from rastai_plugin.job_control import JobControl
from rastai_plugin.job_events import JobEvents
from rastai_plugin.manifest import manifest_ids, read_manifest
from rastai_plugin.module_loader import load_plugin_module
from rastai_plugin.registry import REGISTRY_KINDS, PluginRegistry
from rastai_plugin.types import (
    Batch,
    EvalCase,
    EvalContext,
    MetricResult,
    ModelOutput,
    StepResult,
    TrainingContext,
)

__all__ = [
    "REGISTRY_KINDS",
    "Batch",
    "EvalCase",
    "EvalContext",
    "JobControl",
    "JobEvents",
    "Metric",
    "MetricResult",
    "ModelOutput",
    "PluginError",
    "PluginHost",
    "PluginRegistry",
    "Recipe",
    "StepResult",
    "TrainingContext",
    "WorkerHost",
    "connect_worker_host",
    "load_plugin_module",
    "manifest_ids",
    "read_manifest",
]
