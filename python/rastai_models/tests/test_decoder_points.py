"""The decoder standard observation points (layout python.md sections 2.1 and 2.2, contracts 5.3), hook-tested on a
small two-block decoder whose nodes carry the standard names.

The design is built as a ModelSpec document and validated by rastai_formats.load_model_spec, as a worker's is. Its
graph is the Gemma-shaped block (normalize, attention, post-norm, residual, normalize, feed-forward, post-norm,
residual), with only the standard nodes observed: the attention node's own point is off, so its sibling points are the
only attention points a hook sees, as they are in the family graphs.
"""

import json

import torch
from rastai_formats.generated.model import ModelSpec, model_spec_from_json
from rastai_models.observation import (
    ATTENTION_PATTERN_OUTPUT,
    CAPTURE_FULL_ATTENTION,
    CAPTURE_LAST_TOKEN_ATTENTION,
    HEAD_OUTPUTS_OUTPUT,
    LAST_TOKEN_ATTENTION_OUTPUT,
    attention_sibling_point,
    block_point,
    decoder_standard_points,
)

import rastai_formats
from rastai_models import CompiledModel, ForwardContext, PointHook, compile_model, init_weights

SEED = 11
LAYER_COUNT = 2
VOCABULARY_SIZE = 11
HIDDEN_SIZE = 8
HEAD_COUNT = 2
KV_HEAD_COUNT = 1
HEAD_DIM = 4
INTERMEDIATE_SIZE = 12
BATCH_SIZE = 2
LENGTH = 5


class RecordingHook(PointHook):
    """Records each point's name and shape, in the order the model reports it, and returns each tensor unchanged."""

    def __init__(self) -> None:
        self.point_names: list[str] = []
        self.shapes: dict[str, tuple[int, ...]] = {}

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        self.point_names.append(point_name)
        self.shapes[point_name] = tuple(tensor.shape)
        return tensor


class ZeroingHook(PointHook):
    """Replaces the tensor at one point with zeros, which is what an ablation does."""

    def __init__(self, point: str) -> None:
        self.point = point

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        return torch.zeros_like(tensor) if point_name == self.point else tensor


def node(node_id: str, op: str, attributes: dict, observe: bool) -> dict:
    return {
        "node_id": node_id,
        "op": op,
        "label": node_id,
        "attributes": attributes,
        "train_mode": "train",
        "observe": observe,
    }


def edge(from_node: str, to_node: str, to_input: int | None = None) -> dict:
    document = {"from_node": from_node, "to_node": to_node}
    if to_input is not None:
        document["to_input"] = to_input
    return document


def normalize_attributes() -> dict:
    return {"size": HIDDEN_SIZE, "scale_offset": True}


def block_nodes(layer_index: int) -> list[dict]:
    """One block's nodes. Only the standard points are observed."""
    prefix = f"blocks.{layer_index}"
    attention_attributes = {
        "variant": "gqa",
        "hidden_size": HIDDEN_SIZE,
        "head_count": HEAD_COUNT,
        "kv_head_count": KV_HEAD_COUNT,
        "head_dim": HEAD_DIM,
        "qk_norm": True,
        "rope": {"base": 10000},
    }
    feed_forward_attributes = {
        "variant": "gated",
        "hidden_size": HIDDEN_SIZE,
        "intermediate_size": INTERMEDIATE_SIZE,
        "activation": "silu",
    }
    return [
        node(f"{prefix}.input_norm", "rms_norm", normalize_attributes(), False),
        node(f"{prefix}.attention", "attention", attention_attributes, False),
        node(f"{prefix}.attention_out", "rms_norm", normalize_attributes(), True),
        node(f"{prefix}.attention_residual", "residual_add", {}, False),
        node(f"{prefix}.pre_feedforward_norm", "rms_norm", normalize_attributes(), False),
        node(f"{prefix}.feedforward", "feed_forward", feed_forward_attributes, False),
        node(f"{prefix}.feedforward_out", "rms_norm", normalize_attributes(), True),
        node(f"{prefix}.residual_out", "residual_add", {}, True),
    ]


def block_edges(layer_index: int, residual_stream: str) -> list[dict]:
    prefix = f"blocks.{layer_index}"
    return [
        edge(residual_stream, f"{prefix}.input_norm"),
        edge(f"{prefix}.input_norm", f"{prefix}.attention"),
        edge(f"{prefix}.attention", f"{prefix}.attention_out"),
        edge(residual_stream, f"{prefix}.attention_residual", 0),
        edge(f"{prefix}.attention_out", f"{prefix}.attention_residual", 1),
        edge(f"{prefix}.attention_residual", f"{prefix}.pre_feedforward_norm"),
        edge(f"{prefix}.pre_feedforward_norm", f"{prefix}.feedforward"),
        edge(f"{prefix}.feedforward", f"{prefix}.feedforward_out"),
        edge(f"{prefix}.attention_residual", f"{prefix}.residual_out", 0),
        edge(f"{prefix}.feedforward_out", f"{prefix}.residual_out", 1),
    ]


def decoder_document() -> dict:
    nodes = [
        node(
            "embedding", "embedding", {"vocab_size": VOCABULARY_SIZE, "embedding_size": HIDDEN_SIZE, "scale": 2.0}, True
        )
    ]
    edges: list[dict] = []
    residual_stream = "embedding"
    for layer_index in range(LAYER_COUNT):
        nodes += block_nodes(layer_index)
        edges += block_edges(layer_index, residual_stream)
        residual_stream = block_point(layer_index, "residual_out")
    nodes.append(node("final_norm_out", "rms_norm", normalize_attributes(), True))
    nodes.append(node("unembed", "unembed", {"hidden_size": HIDDEN_SIZE, "vocab_size": VOCABULARY_SIZE}, False))
    edges.append(edge(residual_stream, "final_norm_out"))
    edges.append(edge("final_norm_out", "unembed"))
    return {
        "$schema": "https://schemas.rastai.dev/rastai.model/1.json",
        "type": "rastai.model",
        "schema_version": 1,
        "created_with": "rastai 0.1.0",
        "id": "doc_01JA2B3C4D5E6F7G8H9JDECODER",
        "content_hash": "",
        "derived_from": [],
        "authors": ["Rastai Research Studio"],
        "license": "Apache-2.0",
        "source_url": "",
        "requires": {},
        "graph": {"nodes": nodes, "edges": edges},
        "inputs": [{"name": "tokens", "range": {"start": 0, "end": LENGTH}, "encoder_node": "embedding"}],
        "outputs": [{"name": "logits", "graph_node": "unembed"}],
        "objective": {"kind": "next_token", "settings": {}},
        "defaults": {"dtype": "fp32", "sequence_length": LENGTH, "batch_size": BATCH_SIZE},
        "weights": [],
        "placement": {},
    }


def decoder_spec() -> ModelSpec:
    """The decoder design as a worker reads it: validated by the wheel, then read into the generated dataclass."""
    validated = rastai_formats.load_model_spec(json.dumps(decoder_document()).encode())
    return model_spec_from_json(json.loads(validated))


def compiled_decoder() -> CompiledModel:
    model = compile_model(decoder_spec(), "cpu", torch.float32)
    init_weights(model, SEED)
    model.eval()
    return model


def sample_tokens() -> torch.Tensor:
    generator = torch.Generator().manual_seed(SEED)
    return torch.randint(0, VOCABULARY_SIZE, (BATCH_SIZE, LENGTH), generator=generator)


def expected_points(attention_points: list[str]) -> list[str]:
    """The points a pass reaches, given the attention node's points of one block (in the order they come)."""
    points = [decoder_standard_points(LAYER_COUNT)[0]]
    for layer_index in range(LAYER_COUNT):
        points += [attention_sibling_point(layer_index, name) for name in attention_points]
        points += [
            block_point(layer_index, "attention_out"),
            block_point(layer_index, "feedforward_out"),
            block_point(layer_index, "residual_out"),
        ]
    points.append("final_norm_out")
    return points


# ---- the names ----


def test_the_standard_points_are_the_bare_node_paths_in_order() -> None:
    assert decoder_standard_points(2) == [
        "embedding",
        "blocks.0.attention_out",
        "blocks.0.feedforward_out",
        "blocks.0.residual_out",
        "blocks.1.attention_out",
        "blocks.1.feedforward_out",
        "blocks.1.residual_out",
        "final_norm_out",
    ]


def test_an_attention_sibling_point_is_the_node_path_and_its_output() -> None:
    assert attention_sibling_point(3, HEAD_OUTPUTS_OUTPUT) == "blocks.3.attention.head_outputs"
    assert attention_sibling_point(3, ATTENTION_PATTERN_OUTPUT) == "blocks.3.attention.attention_pattern"
    assert attention_sibling_point(3, LAST_TOKEN_ATTENTION_OUTPUT) == "blocks.3.attention.last_token_attention"


# ---- what a hook sees ----


def test_a_default_pass_shows_the_standard_points_and_the_head_outputs() -> None:
    hook = RecordingHook()
    compiled_decoder()(sample_tokens(), context=ForwardContext(hook=hook))
    assert hook.point_names == expected_points([HEAD_OUTPUTS_OUTPUT])


def test_full_capture_shows_the_pattern_before_the_head_outputs() -> None:
    hook = RecordingHook()
    context = ForwardContext(attention_capture=CAPTURE_FULL_ATTENTION, hook=hook)
    compiled_decoder()(sample_tokens(), context=context)
    assert hook.point_names == expected_points([ATTENTION_PATTERN_OUTPUT, HEAD_OUTPUTS_OUTPUT])


def test_last_token_capture_shows_its_pattern_before_the_head_outputs() -> None:
    hook = RecordingHook()
    context = ForwardContext(attention_capture=CAPTURE_LAST_TOKEN_ATTENTION, hook=hook)
    compiled_decoder()(sample_tokens(), context=context)
    assert hook.point_names == expected_points([LAST_TOKEN_ATTENTION_OUTPUT, HEAD_OUTPUTS_OUTPUT])


def test_point_names_lists_every_point_a_pass_can_emit_in_order() -> None:
    names = compiled_decoder().point_names()
    expected = ["embedding"]
    for layer_index in range(LAYER_COUNT):
        expected += [
            attention_sibling_point(layer_index, ATTENTION_PATTERN_OUTPUT),
            attention_sibling_point(layer_index, LAST_TOKEN_ATTENTION_OUTPUT),
            attention_sibling_point(layer_index, HEAD_OUTPUTS_OUTPUT),
            block_point(layer_index, "attention_out"),
            block_point(layer_index, "feedforward_out"),
            block_point(layer_index, "residual_out"),
        ]
    expected.append("final_norm_out")
    assert names == expected


def test_the_points_have_the_shapes_of_their_outputs() -> None:
    hook = RecordingHook()
    context = ForwardContext(attention_capture=CAPTURE_FULL_ATTENTION, hook=hook)
    compiled_decoder()(sample_tokens(), context=context)
    stream_shape = (BATCH_SIZE, LENGTH, HIDDEN_SIZE)
    assert hook.shapes["embedding"] == stream_shape
    assert hook.shapes[block_point(1, "residual_out")] == stream_shape
    assert hook.shapes[attention_sibling_point(0, ATTENTION_PATTERN_OUTPUT)] == (BATCH_SIZE, HEAD_COUNT, LENGTH, LENGTH)
    assert hook.shapes[attention_sibling_point(0, HEAD_OUTPUTS_OUTPUT)] == (BATCH_SIZE, HEAD_COUNT, LENGTH, HEAD_DIM)


# ---- hooks and the graph ----


def test_a_reader_that_changes_nothing_leaves_the_logits_bit_identical() -> None:
    model = compiled_decoder()
    tokens = sample_tokens()
    plain = model(tokens)
    observed = model(tokens, context=ForwardContext(hook=RecordingHook()))
    assert torch.equal(plain, observed)


def test_an_edit_at_a_block_point_changes_the_graph_after_it() -> None:
    model = compiled_decoder()
    tokens = sample_tokens()
    plain = model(tokens)
    edited = model(tokens, context=ForwardContext(hook=ZeroingHook(block_point(0, "residual_out"))))
    assert not torch.allclose(plain, edited)


def test_an_edit_at_an_attention_sibling_changes_the_graph_after_it() -> None:
    model = compiled_decoder()
    tokens = sample_tokens()
    plain = model(tokens)
    edited = model(tokens, context=ForwardContext(hook=ZeroingHook(attention_sibling_point(0, HEAD_OUTPUTS_OUTPUT))))
    assert not torch.allclose(plain, edited)
