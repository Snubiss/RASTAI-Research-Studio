"""Plugin mode: `python -m rastai_worker plugin --plugin <plugin_id> --job-file <path>` (contracts 3.1; python
layout 7.4 and 7.6).

A plugin with a `python` entry runs as a long-lived `plugin_module` job while the plugin is enabled. This is the
skeleton of worker protocol v0: the process starts like any job (its own log, WorkerService on its socket,
Register) and serves until STOP, CANCEL or SIGTERM. Importing the manifest's module with rastai_plugin's
`load_plugin_module` and serving `HandleCommand` arrive with the HandleCommand RPC and job.json's `plugins[]`
(M4); the node writes no plugin_module job before then.
"""

import time
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_J2001
from rastai_proto.node.v1.jobs_pb2 import JOB_KIND_PLUGIN_MODULE, JobSpec

from rastai_plugin import JobControl, JobEvents, WorkerHost
from rastai_worker.control import install_control, job_is_ending
from rastai_worker.errors import WorkerError
from rastai_worker.exit_codes import EXIT_BAD_LAUNCH
from rastai_worker.job_file import WorkerLaunch, read_job_file
from rastai_worker.launch import report_start_failure, run_launched_job

# How often an idle plugin module looks at its control flags.
IDLE_POLL_SECONDS = 0.1


def run_plugin_process(plugin_id: str, job_file: Path) -> int:
    """Runs a plugin's module job until it is stopped, and returns the exit code."""
    control = install_control()
    try:
        launch = read_job_file(job_file)
        check_plugin_job(launch, plugin_id)
    except WorkerError as error:
        return report_start_failure(error, EXIT_BAD_LAUNCH)
    return run_launched_job(launch, serve_until_stopped, control)


def check_plugin_job(launch: WorkerLaunch, plugin_id: str) -> None:
    """The job.json is a plugin_module job of this plugin: a mismatch means the node started the wrong file."""
    job_spec = launch.job_spec
    if job_spec.kind != JOB_KIND_PLUGIN_MODULE or job_spec.plugin_id != plugin_id:
        reason = f"plugin mode for {plugin_id} needs a plugin_module job of that plugin; {launch.job_file} isn't one"
        raise WorkerError(RRS_J2001, reason)


def serve_until_stopped(job: JobSpec, host: WorkerHost, events: JobEvents, control: JobControl) -> None:
    """The plugin module's entry: nothing to compute until HandleCommand lands, so it waits for the end."""
    while not job_is_ending(control):
        time.sleep(IDLE_POLL_SECONDS)
