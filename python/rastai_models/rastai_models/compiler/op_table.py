"""Which OpModule class runs which ModelSpec op (layout python.md section 2.1).

The table is keyed by (op, variant). The TextCNN ops have no variants; the attention variants (`mha`, `gqa`,
`sliding`, `mla`, contracts 5.3) join with the decoder blocks (M2), and an op the table lacks will be looked up as a
NodeType plugin (M6). Until then an op without an entry is RRS-M0001: the design can be shown but not run.

Every op here has a golden file, `fixtures/ops/<op>.json`, which tests/test_op_fixtures.py checks this table
against: an op added here without its file fails that test (contracts 2.4 rule 10).
"""

from rastai_formats.generated.model import GraphNode
from rastai_plugin.catalog.generated import RRS_M0001

from rastai_models.blocks.basic_ops import Activation, Concat, Dropout
from rastai_models.blocks.convolution import Conv1d, Pool
from rastai_models.blocks.embedding import Embedding
from rastai_models.blocks.heads import ClassifierHead
from rastai_models.blocks.linear import Linear
from rastai_models.blocks.op_module import OpModule
from rastai_models.errors import ModelError

# The variant of an op that has only one.
NO_VARIANT = ""
# The attribute that picks an op's variant, for the ops that have several (contracts 5.3).
VARIANT_ATTRIBUTE = "variant"

OP_MODULE_CLASSES: dict[tuple[str, str], type[OpModule]] = {
    ("embedding", NO_VARIANT): Embedding,
    ("conv1d", NO_VARIANT): Conv1d,
    ("pool", NO_VARIANT): Pool,
    ("linear", NO_VARIANT): Linear,
    ("dropout", NO_VARIANT): Dropout,
    ("activation", NO_VARIANT): Activation,
    ("concat", NO_VARIANT): Concat,
    ("classifier_head", NO_VARIANT): ClassifierHead,
}


def find_op_module_class(node: GraphNode) -> type[OpModule]:
    """The class that runs `node`. Raises ModelError RRS-M0001 for an op (or variant) with no implementation."""
    attributes = node.attributes or {}
    variant = attributes.get(VARIANT_ATTRIBUTE, NO_VARIANT)
    op_class = OP_MODULE_CLASSES.get((node.op, variant))
    if op_class is None:
        shown_op = node.op if variant == NO_VARIANT else f"{node.op}, {variant}"
        message = f"The graph node {node.node_id} ({shown_op}) has no implementation, so the design is view only."
        raise ModelError(RRS_M0001, message)
    return op_class


def implemented_ops() -> list[str]:
    """Every op name the table has an entry for, sorted."""
    return sorted({op for op, _variant in OP_MODULE_CLASSES})
