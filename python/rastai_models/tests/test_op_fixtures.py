"""Every op's numbers against its golden file (layout python.md section 2.7; contracts 2.4 rule 10).

For each `fixtures/ops/<op>.json` case, the op module the compiler builds from the case's attributes gives the
golden output shapes, parameter count and FLOPs (counted by torch.utils.flop_counter.FlopCounterMode, the convention
each file states). Rust live stats match the same files. The test walks the compiler's op table, so an op added to
the table without a golden file fails, and so does a golden file for an op the table lacks.
"""

import json
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.model import GraphNode
from rastai_models.compiler.op_table import find_op_module_class, implemented_ops
from torch.utils.flop_counter import FlopCounterMode

OPS_FOLDER_NAME = "ops"
SEED = 35
# The op whose input holds token ids rather than values.
TOKEN_ID_OPS = {"embedding"}
# The FLOP convention every golden file states word for word (fixtures/ops/README.md).
FLOP_CONVENTION = (
    "flops counts one forward pass over input_shapes, as torch.utils.flop_counter.FlopCounterMode counts it: 2 per "
    "multiply-accumulate of a matrix multiplication or convolution. Bias adds, activations, pooling, dropout, "
    "embedding lookups and concatenation count 0."
)


def golden_files(fixtures_folder: Path) -> dict[str, Path]:
    return {path.stem: path for path in sorted((fixtures_folder / OPS_FOLDER_NAME).glob("*.json"))}


def golden_cases() -> list:
    """(op, case) for every case of every golden file, named `<op>-<case name>`. Collected at import time, so each case
    is its own test."""
    fixtures_folder = Path(__file__).resolve().parents[3] / "fixtures"
    cases = []
    for op, path in golden_files(fixtures_folder).items():
        golden = json.loads(path.read_text(encoding="utf-8"))
        for case in golden["cases"]:
            cases.append(pytest.param(op, case, id=f"{op}-{case['name']}"))
    return cases


def make_inputs(op: str, case: dict, generator: torch.Generator) -> list[torch.Tensor]:
    if op in TOKEN_ID_OPS:
        vocab_size = case["attributes"]["vocab_size"]
        return [torch.randint(0, vocab_size, shape, generator=generator) for shape in case["input_shapes"]]
    return [torch.randn(shape, generator=generator) for shape in case["input_shapes"]]


def test_every_op_in_the_table_has_a_golden_file(fixtures_folder: Path) -> None:
    files = golden_files(fixtures_folder)
    without_file = [op for op in implemented_ops() if op not in files]
    assert without_file == [], f"ops in op_table.py without fixtures/ops/<op>.json: {without_file}"


def test_every_golden_file_states_the_flop_convention(fixtures_folder: Path) -> None:
    for op, path in golden_files(fixtures_folder).items():
        golden = json.loads(path.read_text(encoding="utf-8"))
        assert golden["op"] == op
        assert golden["flop_convention"] == FLOP_CONVENTION


@pytest.mark.parametrize(("op", "case"), golden_cases())
def test_op_module_matches_its_golden_case(op: str, case: dict) -> None:
    node = GraphNode(node_id=case["name"], op=op, attributes=case["attributes"])
    op_module = find_op_module_class(node)(case["attributes"], "cpu", torch.float32)
    # Eval mode, as the goldens were written: dropout passes values through.
    op_module.eval()
    inputs = make_inputs(op, case, torch.Generator().manual_seed(SEED))

    with torch.no_grad(), FlopCounterMode(display=False) as flop_counter:
        output = op_module(*inputs)

    assert [list(output.shape)] == case["output_shapes"]
    assert sum(parameter.numel() for parameter in op_module.parameters()) == case["parameter_count"]
    assert flop_counter.get_total_flops() == case["flops"]
