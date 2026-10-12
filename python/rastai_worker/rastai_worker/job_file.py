"""job.json, what the worker reads at start (contracts 8.4; python layout 7.3).

The node writes `runs/<run_id>/job.json` before it spawns the worker (rastai-node `jobs/job_file.rs`). The worker
reads only the fields below and ignores the rest (the process identity and `job_token_hash` are the node's, for
reattach). The job token itself is never in the file: it arrives in `RASTAI_JOB_TOKEN`.

A file that can't be read, isn't a JSON object, or lacks a field the worker needs is a bad launch, `RRS-J2001`: the
job can't start, and the node shows the reason from the last line of stderr.log.
"""

import json
from dataclasses import dataclass
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_J2001
from rastai_proto.node.v1.jobs_pb2 import JobSpec

from rastai_worker.errors import WorkerError
from rastai_worker.event_payloads import job_spec_from_dict

# Text fields the worker can't run without.
REQUIRED_TEXT_FIELDS = ("job_id", "run_id", "job_socket_path", "worker_host_socket_path", "run_folder")
# Text fields that may be missing or empty: the node fills each one when the task that needs it lands (uv
# environments in M2, cache_folder in M5, resume_from with train.resume).
OPTIONAL_TEXT_FIELDS = ("cache_folder", "environment_id", "environment_lock_hash", "resume_from")
# Fields that are JSON objects or lists, with what an absent one reads as.
OBJECT_FIELDS = ("job_spec", "hardware_profile", "resolved_models")
LIST_FIELDS = ("granted_leases", "plugins")
# The run's event log, in the run folder (contracts 8.4).
EVENT_LOG_FILE_NAME = "events.log"


@dataclass(frozen=True)
class WorkerLaunch:
    """What a job's worker knows about itself, read once from job.json."""

    job_file: Path
    job_id: str
    run_id: str
    # The JobSpec the daemon sent: kind, arguments_json with resolved settings, entry, resources, plugin_id.
    job_spec: JobSpec
    # Where to serve WorkerService, and where the node's WorkerHostService is.
    job_socket_path: str
    worker_host_socket_path: str
    run_folder: Path
    cache_folder: str
    environment_id: str
    environment_lock_hash: str
    # The node's HardwareProfile as protobuf JSON, for the hardware fingerprint (M2).
    hardware_profile: dict
    # {gpu_index, vram_bytes, lease_class} per GPU, for the memory fraction (M2).
    granted_leases: list
    # model_ref -> {model_folder, model_hash, adapter_file, spec_file} (M3).
    resolved_models: dict
    # {plugin_id, version, module, contributes} of every plugin whose contributions the job may load (M4).
    plugins: list
    # A checkpoint folder to resume from, or empty.
    resume_from: str

    @property
    def event_log_path(self) -> Path:
        """The run's event log: runs/<run_id>/events.log (contracts 8.4)."""
        return self.run_folder / EVENT_LOG_FILE_NAME


def read_job_file(job_file: Path) -> WorkerLaunch:
    """The launch that job.json describes. Any problem with the file is RRS-J2001."""
    fields = read_json_object(job_file)
    text = read_text_fields(job_file, fields)
    objects = read_container_fields(job_file, fields, OBJECT_FIELDS, dict)
    lists = read_container_fields(job_file, fields, LIST_FIELDS, list)
    job_spec = read_job_spec(job_file, objects["job_spec"])
    check_job_spec_matches(job_file, job_spec, text["job_id"], text["run_id"])
    return WorkerLaunch(
        job_file=job_file,
        job_id=text["job_id"],
        run_id=text["run_id"],
        job_spec=job_spec,
        job_socket_path=text["job_socket_path"],
        worker_host_socket_path=text["worker_host_socket_path"],
        run_folder=Path(text["run_folder"]),
        cache_folder=text["cache_folder"],
        environment_id=text["environment_id"],
        environment_lock_hash=text["environment_lock_hash"],
        hardware_profile=objects["hardware_profile"],
        granted_leases=lists["granted_leases"],
        resolved_models=objects["resolved_models"],
        plugins=lists["plugins"],
        resume_from=text["resume_from"],
    )


def read_json_object(job_file: Path) -> dict:
    try:
        fields = json.loads(job_file.read_text(encoding="utf-8"))
    except OSError as error:
        raise job_file_error(job_file, f"it can't be read: {error.strerror or error}") from error
    except ValueError as error:
        raise job_file_error(job_file, f"it isn't JSON: {error}") from error
    if not isinstance(fields, dict):
        raise job_file_error(job_file, "it isn't a JSON object")
    return fields


def read_text_fields(job_file: Path, fields: dict) -> dict[str, str]:
    text = {}
    for name in REQUIRED_TEXT_FIELDS:
        value = fields.get(name)
        if not isinstance(value, str) or not value:
            raise job_file_error(job_file, f"it has no {name}")
        text[name] = value
    for name in OPTIONAL_TEXT_FIELDS:
        value = fields.get(name, "")
        if not isinstance(value, str):
            raise job_file_error(job_file, f"its {name} isn't text")
        text[name] = value
    return text


def read_container_fields(job_file: Path, fields: dict, names: tuple[str, ...], kind: type) -> dict:
    """Each named field, which must be a JSON object (kind dict) or list (kind list); a missing one reads empty."""
    values = {}
    for name in names:
        value = fields.get(name, kind())
        if not isinstance(value, kind):
            expected = "an object" if kind is dict else "a list"
            raise job_file_error(job_file, f"its {name} isn't {expected}")
        values[name] = value
    return values


def read_job_spec(job_file: Path, values: dict) -> JobSpec:
    try:
        return job_spec_from_dict(values)
    except ValueError as error:
        raise job_file_error(job_file, str(error)) from error


def check_job_spec_matches(job_file: Path, job_spec: JobSpec, job_id: str, run_id: str) -> None:
    """The spec inside the file is the same job as the file: a mismatch means a damaged or mixed-up file, and the
    worker would serve one job's socket while writing another run's log."""
    if job_spec.job_id != job_id or job_spec.run_id != run_id:
        reason = f"its job_spec is for job {job_spec.job_id} of run {job_spec.run_id}, not job {job_id} of run {run_id}"
        raise job_file_error(job_file, reason)


def job_file_error(job_file: Path, reason: str) -> WorkerError:
    return WorkerError(RRS_J2001, f"{job_file} is unusable: {reason}")
