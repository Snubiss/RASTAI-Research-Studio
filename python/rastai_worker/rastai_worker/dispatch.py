"""Dispatch: a job's kind to the function that runs it (python layout 7.4).

`JOB_ENTRIES` names each core Python job kind's entry as `<module>:<function>`, the same form as `JobSpec.entry`
(contracts 4.4), which names the entry of an `eval` or `lens` job. The module is imported only when a job of that
kind starts, for two reasons: an engine pulls in torch and its kernels, so a job imports only its own kind's engine;
and the job modules land one build step at a time (python layout 7.11, `jobs/train.py` with M1-46), so a kind whose
module isn't in this worker yet is a clear bad launch, not an import error that stops every other kind.

Every entry has the one job entry shape (contracts 12.3; python layout 1.5):

    def run_job(job: JobSpec, host: WorkerHost, events: JobEvents, control: JobControl) -> None
"""

import importlib

from rastai_plugin.catalog.generated import RRS_J2001
from rastai_proto.node.v1.jobs_pb2 import JobKind, JobSpec

from rastai_worker.diagnostics import JobEntry
from rastai_worker.errors import WorkerError

# Core job kind (contracts 9.1) -> its entry. "eval" and "lens" always carry JobSpec.entry instead, and
# "plugin_module" runs in plugin mode (plugin_module.py).
JOB_ENTRIES = {
    "serve_inspectable": "rastai_worker.jobs.serve_inspectable:run_job",
    "train": "rastai_worker.jobs.train:run_job",
    "gym": "rastai_worker.jobs.gym:run_job",
    "data_import": "rastai_worker.jobs.data_import:run_job",
    "data_check": "rastai_worker.jobs.data_check:run_job",
    "compile": "rastai_worker.jobs.compile_design:run_job",
    "export": "rastai_worker.jobs.export:run_job",
    "doctor_selftest": "rastai_worker.jobs.doctor_selftest:run_job",
}
# Kinds whose entry is the JobSpec's own, a plugin's job function (python layout 7.4).
PLUGIN_ENTRY_KINDS = ("eval", "lens")
PLUGIN_MODULE_KIND = "plugin_module"
# JOB_KIND_TRAIN is the kind "train" (contracts 9.1 spells kinds in lower case without the prefix).
JOB_KIND_PREFIX = "JOB_KIND_"
ENTRY_SEPARATOR = ":"


def job_kind_name(job_spec: JobSpec) -> str:
    """The job's kind as contracts 9.1 spells it ("train"), or "" for a number this worker doesn't know."""
    if job_spec.kind not in JobKind.values():
        return ""
    return JobKind.Name(job_spec.kind).removeprefix(JOB_KIND_PREFIX).lower()


def entry_name(job_spec: JobSpec) -> str:
    """The `<module>:<function>` that runs this job in run mode. A kind that doesn't run that way is RRS-J2001."""
    kind = job_kind_name(job_spec)
    if kind in PLUGIN_ENTRY_KINDS:
        if not job_spec.entry:
            raise dispatch_error(job_spec, f"its {kind} job names no entry")
        return job_spec.entry
    if kind == PLUGIN_MODULE_KIND:
        raise dispatch_error(job_spec, "a plugin_module job runs in plugin mode, not run mode")
    if kind not in JOB_ENTRIES:
        raise dispatch_error(job_spec, f"job kind {kind or job_spec.kind} isn't a Python worker kind")
    return JOB_ENTRIES[kind]


def find_job_entry(job_spec: JobSpec) -> JobEntry:
    """Imports the job's entry module and returns its entry function. Anything that stops that (a module this
    worker doesn't have, one that fails to import, a missing function) is RRS-J2001: the job can't start."""
    name = entry_name(job_spec)
    module_name, function_name = split_entry_name(job_spec, name)
    try:
        module = importlib.import_module(module_name)
    except ModuleNotFoundError as error:
        raise dispatch_error(job_spec, f"{error.name or module_name} isn't installed in this worker") from error
    except Exception as error:
        reason = f"importing {module_name} raised {type(error).__name__}: {error}"
        raise dispatch_error(job_spec, reason) from error
    entry = getattr(module, function_name, None)
    if not callable(entry):
        raise dispatch_error(job_spec, f"{module_name} has no function {function_name}")
    return entry


def split_entry_name(job_spec: JobSpec, name: str) -> tuple[str, str]:
    module_name, separator, function_name = name.partition(ENTRY_SEPARATOR)
    if not separator or not module_name or not function_name:
        raise dispatch_error(job_spec, f"its entry {name!r} isn't <module>:<function>")
    return module_name, function_name


def dispatch_error(job_spec: JobSpec, reason: str) -> WorkerError:
    return WorkerError(RRS_J2001, f"job {job_spec.job_id} can't run in this worker: {reason}")
