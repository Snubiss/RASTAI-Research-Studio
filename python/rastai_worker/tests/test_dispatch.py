"""Dispatch (dispatch.py; python layout 7.4): every Python job kind has an entry, and a job this worker can't run is
a bad launch, RRS-J2001, never an import error that escapes.
"""

import sys
import types

import pytest
from rastai_plugin.catalog.generated import RRS_J2001
from rastai_proto.node.v1.jobs_pb2 import JobKind, JobSpec
from rastai_worker.dispatch import JOB_ENTRIES, PLUGIN_ENTRY_KINDS, entry_name, find_job_entry, job_kind_name
from rastai_worker.errors import WorkerError

# The Python worker kinds of contracts 9.1; the other kinds run natively on the node or inside the daemon.
PYTHON_KINDS = (
    "serve_inspectable",
    "train",
    "eval",
    "gym",
    "lens",
    "data_import",
    "data_check",
    "compile",
    "export",
    "doctor_selftest",
    "plugin_module",
)
NODE_NATIVE_KINDS = ("download", "transfer", "serve_fast", "daemon_task")
# A module the tests put in sys.modules, so an entry can be found without a file.
FAKE_MODULE_NAME = "rastai_test_fake_entries"


def job_spec_of(kind_name: str, entry: str = "") -> JobSpec:
    kind = JobKind.Value("JOB_KIND_" + kind_name.upper())
    return JobSpec(job_id="job_a", run_id="run_a", kind=kind, entry=entry)


def recorded_entry(job, host, events, control) -> None:
    """A job entry with the one entry shape (contracts 12.3)."""


@pytest.fixture
def fake_entry_module(monkeypatch: pytest.MonkeyPatch) -> types.ModuleType:
    module = types.ModuleType(FAKE_MODULE_NAME)
    module.run_job = recorded_entry
    module.not_a_function = 3
    monkeypatch.setitem(sys.modules, FAKE_MODULE_NAME, module)
    return module


def test_every_python_job_kind_has_an_entry() -> None:
    for kind in PYTHON_KINDS:
        assert job_kind_name(job_spec_of(kind)) == kind
        if kind in PLUGIN_ENTRY_KINDS or kind == "plugin_module":
            continue
        module_name, function_name = JOB_ENTRIES[kind].split(":")
        assert module_name.startswith("rastai_worker.jobs.")
        assert function_name == "run_job"
    assert sorted(JOB_ENTRIES) == sorted(set(PYTHON_KINDS) - set(PLUGIN_ENTRY_KINDS) - {"plugin_module"})


@pytest.mark.parametrize("kind", NODE_NATIVE_KINDS + ("unspecified",))
def test_a_kind_that_isnt_a_python_worker_kind_is_refused(kind: str) -> None:
    with pytest.raises(WorkerError) as raised:
        entry_name(job_spec_of(kind))
    assert raised.value.code == RRS_J2001


def test_an_unknown_kind_number_is_refused() -> None:
    job_spec = JobSpec(job_id="job_a", kind=99)
    assert job_kind_name(job_spec) == ""
    with pytest.raises(WorkerError) as raised:
        entry_name(job_spec)
    assert "99" in raised.value.message


def test_a_plugin_module_job_runs_only_in_plugin_mode() -> None:
    with pytest.raises(WorkerError) as raised:
        entry_name(job_spec_of("plugin_module"))
    assert "plugin mode" in raised.value.message


@pytest.mark.parametrize("kind", PLUGIN_ENTRY_KINDS)
def test_eval_and_lens_jobs_use_their_own_entry(kind: str) -> None:
    assert entry_name(job_spec_of(kind, "rastai_eval.suite_job:run_suite_job")) == "rastai_eval.suite_job:run_suite_job"
    with pytest.raises(WorkerError) as raised:
        entry_name(job_spec_of(kind))
    assert "names no entry" in raised.value.message


def test_the_entry_function_is_found(fake_entry_module: types.ModuleType) -> None:
    assert find_job_entry(job_spec_of("eval", f"{FAKE_MODULE_NAME}:run_job")) is recorded_entry


def test_a_core_kind_whose_module_isnt_built_yet_is_a_bad_launch() -> None:
    """The job modules land one build step at a time (python layout 7.11): until then the kind is refused with a
    reason, and the worker never crashes on the import."""
    with pytest.raises(WorkerError) as raised:
        find_job_entry(job_spec_of("serve_inspectable"))
    assert raised.value.code == RRS_J2001
    assert "isn't installed in this worker" in raised.value.message


@pytest.mark.parametrize(
    ("entry", "reason"),
    [
        ("no_such_module_anywhere:run_job", "no_such_module_anywhere isn't installed"),
        (f"{FAKE_MODULE_NAME}:missing", "has no function missing"),
        (f"{FAKE_MODULE_NAME}:not_a_function", "has no function not_a_function"),
        (f"{FAKE_MODULE_NAME}", "isn't <module>:<function>"),
        (":run_job", "isn't <module>:<function>"),
        ("../escape:run_job", "importing ../escape raised"),
    ],
)
def test_an_entry_that_cant_be_found_is_a_bad_launch(
    fake_entry_module: types.ModuleType, entry: str, reason: str
) -> None:
    with pytest.raises(WorkerError) as raised:
        find_job_entry(job_spec_of("eval", entry))
    assert raised.value.code == RRS_J2001
    assert reason in raised.value.message


def test_an_entry_module_that_crashes_on_import_is_a_bad_launch(monkeypatch: pytest.MonkeyPatch, tmp_path) -> None:
    (tmp_path / "rastai_test_crashing_entry.py").write_text("raise RuntimeError('broken at import')\n")
    monkeypatch.syspath_prepend(str(tmp_path))
    with pytest.raises(WorkerError) as raised:
        find_job_entry(job_spec_of("lens", "rastai_test_crashing_entry:run_job"))
    assert raised.value.code == RRS_J2001
    assert "RuntimeError: broken at import" in raised.value.message
