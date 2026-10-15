"""ModelTraits: what a compiled design contains, read from its ModelSpec (layout python.md sections 2.1 and 2.4).

The runtime and the readouts ask these questions before they run: does the model have experts to route, a latent
cache, ALiBi bias, depth attention (which makes the lens and SAE readouts approximate), and which attention layers
are sliding. The traits are derived from the spec's ops, so a design and an imported model give the same answers.

Block bodies (`spec.blocks`) count for the has_* traits: a block instance hides its body's nodes from the top-level
graph, and a design with a Mixture-of-Experts block still has experts. The sliding ids are the exception. A body's node
has no graph path until block expansion defines instance paths, so its id would repeat across bodies and across the
stamped copies; the ring cache can only use ids that name one node.
"""

from dataclasses import dataclass

from rastai_formats.generated.model import GraphNode, ModelSpec

from rastai_models.compiler.op_table import VARIANT_ATTRIBUTE

# The ops that make a model Mixture-of-Experts (contracts 5.3): the container and its router and experts.
MOE_OPS = ("moe", "moe_router", "moe_expert")
ATTENTION_OP = "attention"
# The attention variant with a compressed latent cache (contracts 5.3).
LATENT_ATTENTION_VARIANT = "mla"
# The attention variant with a fixed window (contracts 5.3).
SLIDING_ATTENTION_VARIANT = "sliding"
ALIBI_OP = "alibi"
ATTN_RES_OP = "attn_res"


@dataclass(frozen=True)
class ModelTraits:
    has_moe: bool = False
    has_mla: bool = False
    has_alibi: bool = False
    has_attn_res: bool = False
    # The ids of the top-level attention nodes with a sliding window, in graph order (the ring cache applies to these).
    # Block-body nodes are left out until block expansion gives instance paths (see the module docstring).
    sliding_node_ids: tuple[str, ...] = ()
    # Depth attention mixes earlier layers' outputs in a way the logit lens and the SAE were not fit to, so their
    # readouts are labeled approximate (layout 2.4).
    approximate_lens: bool = False


def model_traits_of(spec: ModelSpec) -> ModelTraits:
    """The traits of `spec`, from its top-level graph and every block body."""
    nodes = all_graph_nodes(spec)
    has_attn_res = any(node.op == ATTN_RES_OP for node in nodes)
    return ModelTraits(
        has_moe=any(node.op in MOE_OPS for node in nodes),
        has_mla=any(is_attention_variant(node, LATENT_ATTENTION_VARIANT) for node in nodes),
        has_alibi=any(node.op == ALIBI_OP for node in nodes),
        has_attn_res=has_attn_res,
        sliding_node_ids=tuple(
            node.node_id for node in spec.graph.nodes if is_attention_variant(node, SLIDING_ATTENTION_VARIANT)
        ),
        approximate_lens=has_attn_res,
    )


def all_graph_nodes(spec: ModelSpec) -> list[GraphNode]:
    """The nodes of the top-level graph, then of each block body in the order the spec lists them."""
    graphs = [spec.graph]
    if spec.blocks:
        graphs.extend(spec.blocks.values())
    nodes = []
    for graph in graphs:
        nodes.extend(graph.nodes)
    return nodes


def is_attention_variant(node: GraphNode, variant: str) -> bool:
    if node.op != ATTENTION_OP:
        return False
    attributes = node.attributes or {}
    return attributes.get(VARIANT_ATTRIBUTE) == variant
