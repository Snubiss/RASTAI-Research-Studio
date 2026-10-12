"""compile_model: a validated ModelSpec → a CompiledModel of plain torch modules (layout python.md section 2.2).

The steps: find each graph node's op module class (RRS-M0001 for one with no implementation), put the nodes in graph
order, build each op module from its attributes, then apply each node's train mode. The parameters start as zeros;
`init_weights` (a fresh design) or `bind_weights` (weights from safetensors files) fills them.

The spec must come through `rastai_formats.load_model_spec`, the one validation (contracts 2.4 rule 10): this file
checks nothing a validated spec already guarantees.
"""

import torch
from rastai_formats.generated.model import GraphNode, ModelSpec

from rastai_models.blocks.op_module import OpModule
from rastai_models.compiler.compiled_model import CompiledModel
from rastai_models.compiler.graph_order import order_graph
from rastai_models.compiler.op_table import find_op_module_class
from rastai_models.compiler.train_modes import apply_train_modes


def compile_model(spec: ModelSpec, device: str | torch.device, dtype: torch.dtype) -> CompiledModel:
    """The design `spec` as a CompiledModel on `device`, its parameters in `dtype` and set to zero."""
    # Looked up before ordering, so a node with no implementation is reported as RRS-M0001 even when it sits on a
    # loop (only an `rnn` node, which has none yet, may close one).
    op_classes = {node.node_id: find_op_module_class(node) for node in spec.graph.nodes}
    graph_steps = order_graph(spec)
    op_modules = [build_op_module(step.node, op_classes[step.node.node_id], device, dtype) for step in graph_steps]
    model = CompiledModel(spec, graph_steps, op_modules)
    apply_train_modes(model)
    return model


def build_op_module(
    node: GraphNode, op_class: type[OpModule], device: str | torch.device, dtype: torch.dtype
) -> OpModule:
    return op_class(node.attributes or {}, device, dtype)
