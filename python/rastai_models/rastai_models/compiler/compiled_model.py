"""CompiledModel: a ModelSpec as one torch module that runs its graph steps in order (layout python.md section 2.2).

The op modules sit in a ModuleList in graph order rather than under their node ids, because a node id is any string
(contracts 5.2) and torch refuses a module name with a dot in it. Their parameters are named by graph path instead,
`<node_id>.<name>` (`graph_parameters`), which is the name weight files use.

The model's input is one sequence, [batch, length]; each input map entry hands the positions in its range to its
encoder node (contracts 5.2). The decoder-only parts of the layout's interface (ForwardContext, hooks, final_norm,
unembed, prepare, point_names) join with the unified runtime (M2); placement is applied by apply_placement (M6) from
the spec's `placement{}` field, which Python never plans (contracts 2.4 rule 10).
"""

import torch
from rastai_formats.generated.model import ModelSpec
from torch import nn

from rastai_models.blocks.op_module import OpModule
from rastai_models.compiler.graph_order import GraphStep, StepInput


class CompiledModel(nn.Module):
    """The compiled graph. `forward` runs every step in order and returns the output of the first head."""

    def __init__(self, spec: ModelSpec, graph_steps: list[GraphStep], op_modules: list[OpModule]) -> None:
        super().__init__()
        self.spec = spec
        self.graph_steps = graph_steps
        self.op_modules = nn.ModuleList(op_modules)

    def forward(self, input_sequence: torch.Tensor) -> torch.Tensor:
        outputs = self.run_steps(input_sequence)
        # TextCNN has one head. Designs with several (a reward head beside the logits) choose theirs in M5.
        return outputs[self.spec.outputs[0].graph_node]

    def run_steps(self, input_sequence: torch.Tensor) -> dict[str, torch.Tensor]:
        """Every graph node's output by node id."""
        outputs: dict[str, torch.Tensor] = {}
        for step, op_module in zip(self.graph_steps, self.op_modules, strict=True):
            step_inputs = [read_step_input(source, input_sequence, outputs) for source in step.inputs]
            outputs[step.node.node_id] = op_module(*step_inputs)
        return outputs

    def graph_parameters(self) -> dict[str, nn.Parameter]:
        """Every parameter by its graph path, `<node_id>.<name>`, in graph order."""
        parameters = {}
        for step, op_module in zip(self.graph_steps, self.op_modules, strict=True):
            for name, parameter in op_module.named_parameters():
                parameters[f"{step.node.node_id}.{name}"] = parameter
        return parameters


def read_step_input(source: StepInput, input_sequence: torch.Tensor, outputs: dict[str, torch.Tensor]) -> torch.Tensor:
    if source.from_node is not None:
        return outputs[source.from_node]
    # Positions are the second axis of a [batch, length] sequence. A sequence shorter than the design's
    # sequence_length gives the range what it has, as slicing does; TextCNN pools over whatever length arrives.
    span = source.input_range
    return input_sequence[:, span.start : span.end]
