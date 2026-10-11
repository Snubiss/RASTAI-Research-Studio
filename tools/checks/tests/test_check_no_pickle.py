"""check-no-pickle on passing and failing fixture trees (contracts 12.3, python layout 1.4 rule 1)."""

import pytest

PASSING_MODULE = '''"""Writes checkpoints. Never pickle, never torch.load: safetensors plus JSON."""

import json

import torch

# torch.save would pickle; this file writes safetensors instead.
BANNED_NOTE = "torch.load and pickle are banned"
# A dynamic import of an allowed module passes.
json_module = __import__("json")


def save_state(model: torch.nn.Module) -> str:
    return json.dumps({"parameters": sum(parameter.numel() for parameter in model.parameters())})
'''

# (file name, source) pairs that each must fail on their own.
FAILING_SOURCES = [
    ("pickle_import.py", "import pickle\n"),
    ("pickle_from_import.py", "from pickle import dumps\n"),
    ("torch_load.py", "import torch\n\nstate = torch.load('model.pt')\n"),
    ("torch_save_alias.py", "import torch as t\n\nt.save({}, 'state.pt')\n"),
    ("torch_load_from_import.py", "from torch import load\n"),
    ("distributed_checkpoint.py", "import torch.distributed.checkpoint as dcp\n"),
    ("object_collective.py", "import torch.distributed as dist\n\ndist.all_gather_object([], 1)\n"),
    ("numpy_pickle.py", "def read(numpy, path):\n    return numpy.load(path, allow_pickle=True)\n"),
    ("process_spawn.py", "import multiprocessing\n"),
    ("process_pool.py", "from concurrent.futures import ProcessPoolExecutor\n"),
    ("import_module.py", "import importlib\n\nloader = importlib.import_module('pickle')\n"),
    ("import_module_from.py", "from importlib import import_module\n\nimport_module('torch.serialization')\n"),
    ("dunder_import.py", "codec = __import__('pickle')\n"),
]


def test_passing_tree(write_tree, run_check):
    root = write_tree(
        {
            "python/rastai_train/rastai_train/checkpoints.py": PASSING_MODULE,
            "python/rastai_train/rastai_train/weights.safetensors": "",
            "spike/bakeoff/python/old.py": "import pickle\n",
            ".dev/envs/cpu/lib/site.py": "import pickle\n",
        }
    )
    result = run_check("check-no-pickle", "--root", str(root))
    assert result.returncode == 0, result.stdout


@pytest.mark.parametrize(("file_name", "source"), FAILING_SOURCES)
def test_each_pickle_use_fails(write_tree, run_check, file_name, source):
    path = f"python/rastai_train/rastai_train/{file_name}"
    root = write_tree({path: source})
    result = run_check("check-no-pickle", "--root", str(root))
    assert result.returncode == 1
    assert f"{path}:" in result.stdout


def test_pickle_format_files_fail(write_tree, run_check):
    root = write_tree({"fixtures/models/tiny/pytorch_model.bin": "", "fixtures/models/tiny/state.pt": ""})
    result = run_check("check-no-pickle", "--root", str(root))
    assert result.returncode == 1
    assert "fixtures/models/tiny/pytorch_model.bin: pickle-format file" in result.stdout
    assert "fixtures/models/tiny/state.pt: pickle-format file" in result.stdout


def test_a_python_script_without_a_py_ending_is_checked(write_tree, run_check):
    root = write_tree({"tools/checks/check-example": "#!/usr/bin/env python3\nimport pickle\n"})
    result = run_check("check-no-pickle", "--root", str(root))
    assert result.returncode == 1
    assert "tools/checks/check-example:2: `pickle`" in result.stdout


def test_the_real_repository_passes(run_check):
    result = run_check("check-no-pickle")
    assert result.returncode == 0, result.stdout
