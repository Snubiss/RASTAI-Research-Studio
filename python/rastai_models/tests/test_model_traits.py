"""ModelTraits (layout python.md section 2.1): the traits come from the spec's ops, top-level graph and block bodies
alike. The TextCNN template has none of the decoder-only traits; a design that has them reports each one.

The specs with the newer ops are built from the TextCNN template's JSON without validation, because the traits read
only the graph's ops and variants, which the validator would not yet accept for ops that have no shapes.
"""

import json
from pathlib import Path

from rastai_formats.generated.model import ModelSpec, model_spec_from_json

from rastai_models import ModelTraits, model_traits_of

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")


def textcnn_document(repo_root: Path) -> dict:
    return json.loads((repo_root / TEXTCNN_TEMPLATE).read_text(encoding="utf-8"))


def spec_from(document: dict) -> ModelSpec:
    return model_spec_from_json(document)


def add_node(document: dict, node_id: str, op: str, attributes: dict | None = None) -> None:
    node = {"node_id": node_id, "op": op}
    if attributes is not None:
        node["attributes"] = attributes
    document["graph"]["nodes"].append(node)


def test_textcnn_has_none_of_the_decoder_traits(repo_root: Path) -> None:
    spec = spec_from(textcnn_document(repo_root))
    assert model_traits_of(spec) == ModelTraits()


def test_a_moe_block_is_mixture_of_experts(repo_root: Path) -> None:
    for op in ("moe", "moe_router", "moe_expert"):
        document = textcnn_document(repo_root)
        add_node(document, "experts", op)
        assert model_traits_of(spec_from(document)).has_moe, op


def test_an_mla_attention_variant_has_latent_attention(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    add_node(document, "latent", "attention", {"variant": "mla"})
    assert model_traits_of(spec_from(document)).has_mla


def test_a_plain_attention_variant_is_not_latent(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    add_node(document, "plain", "attention", {"variant": "gqa"})
    assert not model_traits_of(spec_from(document)).has_mla


def test_an_alibi_node_has_alibi(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    add_node(document, "bias", "alibi")
    assert model_traits_of(spec_from(document)).has_alibi


def test_attention_residual_makes_the_lens_approximate(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    add_node(document, "depth", "attn_res")
    traits = model_traits_of(spec_from(document))
    assert traits.has_attn_res
    assert traits.approximate_lens


def test_sliding_attention_nodes_are_listed_in_graph_order(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    add_node(document, "sliding_first", "attention", {"variant": "sliding", "window": 512})
    add_node(document, "full_middle", "attention", {"variant": "gqa"})
    add_node(document, "sliding_last", "attention", {"variant": "sliding", "window": 512})
    traits = model_traits_of(spec_from(document))
    assert traits.sliding_node_ids == ("sliding_first", "sliding_last")


def test_a_block_body_sliding_node_is_not_listed(repo_root: Path) -> None:
    # A body's node has no graph path yet, so its id could repeat across bodies and stamped copies.
    document = textcnn_document(repo_root)
    body = {
        "nodes": [{"node_id": "attention", "op": "attention", "attributes": {"variant": "sliding", "window": 512}}],
        "edges": [],
    }
    document["blocks"] = {"local_layer": body}
    assert model_traits_of(spec_from(document)).sliding_node_ids == ()


def test_a_block_body_counts_for_the_traits(repo_root: Path) -> None:
    document = textcnn_document(repo_root)
    body = {
        "nodes": [{"node_id": "body_experts", "op": "moe_router"}, {"node_id": "body_bias", "op": "alibi"}],
        "edges": [],
    }
    document["blocks"] = {"block_one": body}
    traits = model_traits_of(spec_from(document))
    assert traits.has_moe
    assert traits.has_alibi
