"""The order a ModelSpec's graph nodes run in (layout python.md section 2.1).

The spec arrives validated by rastai-formats (`rastai_formats.load_model_spec`): every edge and input map entry names
a graph node, every input of a TextCNN op is fed exactly once (RRS-E2007), and a loop needs an `rnn` node (RRS-E2003).
So this file only puts the nodes in an order where each runs after everything that feeds it. Ties go to the node
listed first in the file, so one spec always gives one order. Block instances and `repeat` are expanded here when
they join `rastai.model/1` (M2-01).
"""

import heapq
from dataclasses import dataclass

from rastai_formats.generated.model import GraphNode, InputSpan, ModelSpec

# An edge without `to_input` feeds input 0 (contracts 5.2).
DEFAULT_TO_INPUT = 0
# An input map entry feeds its encoder node's input 0 (contracts 5.2).
ENCODER_INPUT = 0


@dataclass(frozen=True)
class StepInput:
    """Where one input of a graph step comes from: the output of graph node `from_node`, or, for an input map entry,
    the positions `input_range` of the model's input sequence. Exactly one of the two is set."""

    from_node: str | None = None
    input_range: InputSpan | None = None


@dataclass(frozen=True)
class GraphStep:
    """One graph node and its inputs in `to_input` order."""

    node: GraphNode
    inputs: list[StepInput]


def order_graph(spec: ModelSpec) -> list[GraphStep]:
    """Every graph node of `spec` as a step, each after the nodes that feed it."""
    inputs_by_node = collect_step_inputs(spec)
    return [GraphStep(node=node, inputs=inputs_by_node[node.node_id]) for node in topological_order(spec)]


def collect_step_inputs(spec: ModelSpec) -> dict[str, list[StepInput]]:
    """Each graph node's inputs, sorted by input number."""
    numbered_inputs: dict[str, dict[int, StepInput]] = {node.node_id: {} for node in spec.graph.nodes}
    for entry in spec.inputs:
        numbered_inputs[entry.encoder_node][ENCODER_INPUT] = StepInput(input_range=entry.range)
    for edge in spec.graph.edges or []:
        to_input = DEFAULT_TO_INPUT if edge.to_input is None else edge.to_input
        numbered_inputs[edge.to_node][to_input] = StepInput(from_node=edge.from_node)
    return {node_id: [inputs[number] for number in sorted(inputs)] for node_id, inputs in numbered_inputs.items()}


def topological_order(spec: ModelSpec) -> list[GraphNode]:
    """Kahn's algorithm: a node is ready once every node feeding it has run. The ready nodes wait in a heap keyed
    by their place in the file, so the earliest-listed ready node always runs next."""
    nodes = spec.graph.nodes
    place_by_id = {node.node_id: place for place, node in enumerate(nodes)}
    feeding_counts = [0] * len(nodes)
    fed_places: list[list[int]] = [[] for _ in nodes]
    for edge in spec.graph.edges or []:
        to_place = place_by_id[edge.to_node]
        feeding_counts[to_place] += 1
        fed_places[place_by_id[edge.from_node]].append(to_place)

    ready = [place for place, count in enumerate(feeding_counts) if count == 0]
    heapq.heapify(ready)
    order = []
    while ready:
        place = heapq.heappop(ready)
        order.append(nodes[place])
        for fed_place in fed_places[place]:
            feeding_counts[fed_place] -= 1
            if feeding_counts[fed_place] == 0:
                heapq.heappush(ready, fed_place)

    if len(order) != len(nodes):
        # Only a loop leaves nodes behind. Validation lets a loop through only beside an `rnn` node, which has no
        # implementation yet, so reaching this means the spec skipped validation: a bug in the caller.
        left = [nodes[place].node_id for place, count in enumerate(feeding_counts) if count > 0]
        raise ValueError(f"the graph has a loop through {left}; compile only a spec rastai_formats validated")
    return order
