"""CompiledModel: a ModelSpec as one torch module that runs its graph steps in order (layout python.md section 2.2).

The op modules sit in a ModuleList in graph order rather than under their node ids, because a node id is any string
(contracts 5.2) and torch refuses a module name with a dot in it. Their parameters are named by graph path instead,
`<node_id>.<name>` (`graph_parameters`), which is the name weight files use.

The model's input is one sequence, [batch, length]; each input map entry hands the positions in its range to its
encoder node (contracts 5.2). Each node's output is an observation point named by the node's id (observation.py): the
hook in the pass's ForwardContext sees every point in graph order and may edit the tensor the graph goes on with. An op
with extra outputs (attention's head outputs and patterns) passes its sibling points to the hook inside its own forward,
before the node's point. A node's `observe` covers only its own output; its sibling points are emitted whenever the
pass has a hook. The decoder-only parts of the layout's interface (final_norm, unembed, prepare) join with the decoder
graphs (M2-05); placement is applied by apply_placement (M6) from the spec's `placement{}` field, which Python never
plans (contracts 2.4 rule 10).
"""

import torch
from rastai_formats.generated.model import GraphNode, ModelSpec
from torch import nn

from rastai_models.blocks.op_module import OpModule
from rastai_models.compiler.graph_order import GraphStep, StepInput
from rastai_models.forward_context import ForwardContext
from rastai_models.model_traits import model_traits_of


class CompiledModel(nn.Module):
    """The compiled graph. `forward` runs every step in order and returns the output of the first head."""

    def __init__(self, spec: ModelSpec, graph_steps: list[GraphStep], op_modules: list[OpModule]) -> None:
        super().__init__()
        self.spec = spec
        self.traits = model_traits_of(spec)
        self.graph_steps = graph_steps
        self.op_modules = nn.ModuleList(op_modules)

    def forward(self, input_sequence: torch.Tensor, *, context: ForwardContext | None = None) -> torch.Tensor:
        outputs = self.run_steps(input_sequence, context=context)
        # TextCNN has one head. Designs with several (a reward head beside the logits) choose theirs in M5.
        return outputs[self.spec.outputs[0].graph_node]

    def run_steps(
        self, input_sequence: torch.Tensor, *, context: ForwardContext | None = None
    ) -> dict[str, torch.Tensor]:
        """Every graph node's output by node id. With a hook in `context`, each observed output goes through it before
        the graph's later steps read it, so an edit at a point reaches every node after it. A pass without a context
        runs the default one (SDPA, nothing captured, no hook)."""
        if context is None:
            context = ForwardContext()
        outputs: dict[str, torch.Tensor] = {}
        for step, op_module in zip(self.graph_steps, self.op_modules, strict=True):
            step_inputs = [read_step_input(source, input_sequence, outputs) for source in step.inputs]
            output = op_module(*step_inputs, context=context)
            if is_observed(step.node) and context.hook is not None:
                output = context.hook.at_point(step.node.node_id, output)
            outputs[step.node.node_id] = output
        return outputs

    def point_names(self) -> list[str]:
        """The observation points a pass can emit, in graph order. A node's sibling points come first, in the order its
        op emits them, and its own point last: the order the hook sees them in. The sibling points are listed whether
        or not the node is observed, and some passes emit only some of them (layout 2.2)."""
        names: list[str] = []
        for step, op_module in zip(self.graph_steps, self.op_modules, strict=True):
            names.extend(f"{step.node.node_id}.{output}" for output in op_module.sibling_points())
            if is_observed(step.node):
                names.append(step.node.node_id)
        return names

    def graph_parameters(self) -> dict[str, nn.Parameter]:
        """Every parameter by its graph path, `<node_id>.<name>`, in graph order."""
        parameters = {}
        for step, op_module in zip(self.graph_steps, self.op_modules, strict=True):
            for name, parameter in op_module.named_parameters():
                parameters[f"{step.node.node_id}.{name}"] = parameter
        return parameters


def is_observed(node: GraphNode) -> bool:
    """Whether a node's output is an observation point. The spec's `observe` is true by default (contracts 5.2), so
    only an explicit false takes a node out."""
    return node.observe is not False


def read_step_input(source: StepInput, input_sequence: torch.Tensor, outputs: dict[str, torch.Tensor]) -> torch.Tensor:
    if source.from_node is not None:
        return outputs[source.from_node]
    # Positions are the second axis of a [batch, length] sequence. A sequence shorter than the design's
    # sequence_length gives the range what it has, as slicing does; TextCNN pools over whatever length arrives.
    span = source.input_range
    return input_sequence[:, span.start : span.end]
