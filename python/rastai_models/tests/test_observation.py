"""The observation contract (layout python.md section 2.2): a PointHook sees every point of a compiled model in graph
order, a hook that returns its tensor unchanged leaves every output bit-identical, and a hook that edits a point
changes the graph after it.

The model is the TextCNN template, which every test here compiles through `rastai_formats.load_model_spec`, as a
worker does.

The decoder standard names (`embedding`, `blocks.<i>.attention_out`, ..., `final_norm_out`) are checked in
test_decoder_points.py, on a two-block decoder built by hand: the family graphs that give them come with M2-05.
"""

import json
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.model import ModelSpec, model_spec_from_json
from rastai_models.attention_kernel import KERNEL_SDPA
from rastai_models.observation import (
    CAPTURE_FULL_ATTENTION,
    CAPTURE_LAST_TOKEN_ATTENTION,
    CAPTURE_MODES,
    CAPTURE_NO_ATTENTION,
    check_capture_mode,
)

import rastai_formats
from rastai_models import CompiledModel, DecodeReadout, ForwardContext, PointHook, compile_model, init_weights

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
# The TextCNN template's graph nodes in graph order: the observation points every hook sees.
TEXTCNN_POINTS = [
    "embedding",
    "conv_1",
    "relu_1",
    "conv_2",
    "relu_2",
    "conv_3",
    "relu_3",
    "conv_4",
    "relu_4",
    "conv_5",
    "relu_5",
    "pool",
    "dropout",
    "classifier",
]
SEED = 2
BATCH_SIZE = 2
SEQUENCE_LENGTH = 128
VOCABULARY_SIZE = 25_000


class RecordingHook(PointHook):
    """Records each point's name in the order the model reports it, and returns each tensor unchanged."""

    def __init__(self) -> None:
        self.point_names: list[str] = []

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        self.point_names.append(point_name)
        return tensor


class ZeroingHook(PointHook):
    """Replaces the tensor at one point with zeros, which is what an ablation does."""

    def __init__(self, point: str) -> None:
        self.point = point

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        if point_name == self.point:
            return torch.zeros_like(tensor)
        return tensor


def read_spec(json_bytes: bytes) -> ModelSpec:
    return model_spec_from_json(json.loads(rastai_formats.load_model_spec(json_bytes)))


def compiled_textcnn(repo_root: Path) -> CompiledModel:
    spec = read_spec((repo_root / TEXTCNN_TEMPLATE).read_bytes())
    model = compile_model(spec, "cpu", torch.float32)
    init_weights(model, SEED)
    model.eval()
    return model


def sample_tokens() -> torch.Tensor:
    generator = torch.Generator().manual_seed(SEED)
    return torch.randint(0, VOCABULARY_SIZE, (BATCH_SIZE, SEQUENCE_LENGTH), generator=generator)


def unobserved_dropout_model(repo_root: Path) -> CompiledModel:
    """The TextCNN template with its dropout node's `observe` set to false."""
    document = json.loads((repo_root / TEXTCNN_TEMPLATE).read_bytes())
    for node in document["graph"]["nodes"]:
        if node["node_id"] == "dropout":
            node["observe"] = False
    model = compile_model(read_spec(json.dumps(document).encode()), "cpu", torch.float32)
    init_weights(model, SEED)
    model.eval()
    return model


def run_with_hook(model: CompiledModel, hook: PointHook) -> dict[str, torch.Tensor]:
    return model.run_steps(sample_tokens(), context=ForwardContext(hook=hook))


# ---- the points and their order ----


def test_hook_sees_every_standard_point_in_graph_order(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    hook = RecordingHook()
    model(sample_tokens(), context=ForwardContext(hook=hook))
    assert hook.point_names == TEXTCNN_POINTS


def test_point_names_are_the_graph_nodes_in_order(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    assert model.point_names() == TEXTCNN_POINTS


def test_an_unobserved_node_is_not_a_point_name(repo_root: Path) -> None:
    model = unobserved_dropout_model(repo_root)
    assert model.point_names() == [point for point in TEXTCNN_POINTS if point != "dropout"]


def test_the_hook_does_not_see_an_unobserved_node(repo_root: Path) -> None:
    model = unobserved_dropout_model(repo_root)
    hook = RecordingHook()
    model(sample_tokens(), context=ForwardContext(hook=hook))
    assert hook.point_names == [point for point in TEXTCNN_POINTS if point != "dropout"]


def test_an_unobserved_node_still_runs(repo_root: Path) -> None:
    model = unobserved_dropout_model(repo_root)
    assert "dropout" in model.run_steps(sample_tokens())


def test_an_edit_at_an_unobserved_node_changes_nothing(repo_root: Path) -> None:
    model = unobserved_dropout_model(repo_root)
    tokens = sample_tokens()
    plain_logits = model(tokens)
    ablated_logits = model(tokens, context=ForwardContext(hook=ZeroingHook("dropout")))
    assert torch.equal(plain_logits, ablated_logits)


def test_every_point_is_seen_once_per_pass(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    hook = RecordingHook()
    model(sample_tokens(), context=ForwardContext(hook=hook))
    assert len(hook.point_names) == len(set(hook.point_names))


# ---- readers change nothing ----


def test_identity_hook_leaves_every_output_bit_identical(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    plain = model.run_steps(sample_tokens())
    hooked = run_with_hook(model, PointHook())
    assert plain.keys() == hooked.keys()
    for node_id in plain:
        assert torch.equal(plain[node_id], hooked[node_id]), node_id


def test_hook_that_copies_each_tensor_leaves_logits_bit_identical(repo_root: Path) -> None:
    class CopyingHook(PointHook):
        def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
            return tensor.clone()

    model = compiled_textcnn(repo_root)
    tokens = sample_tokens()
    plain_logits = model(tokens)
    copied_logits = model(tokens, context=ForwardContext(hook=CopyingHook()))
    assert torch.equal(plain_logits, copied_logits)


def test_base_point_hook_returns_its_tensor(repo_root: Path) -> None:
    tensor = torch.arange(6, dtype=torch.float32)
    assert PointHook().at_point("embedding", tensor) is tensor


# ---- interventions change the graph after their point ----


def test_editing_a_point_changes_the_logits(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    tokens = sample_tokens()
    plain_logits = model(tokens)
    ablated_logits = model(tokens, context=ForwardContext(hook=ZeroingHook("pool")))
    assert not torch.equal(plain_logits, ablated_logits)


def test_editing_the_head_point_sets_the_logits_it_returns(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    logits = model(sample_tokens(), context=ForwardContext(hook=ZeroingHook("classifier")))
    assert torch.equal(logits, torch.zeros_like(logits))


def test_a_later_point_sees_the_edited_tensor(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    hook = ZeroingHook("relu_1")
    outputs = run_with_hook(model, hook)
    assert torch.equal(outputs["relu_1"], torch.zeros_like(outputs["relu_1"]))
    # conv_2 reads relu_1, so with relu_1 zeroed the conv_2 output is its bias everywhere.
    conv_2_bias = model.op_modules[model.point_names().index("conv_2")].bias
    expected = conv_2_bias.expand_as(outputs["conv_2"])
    assert torch.allclose(outputs["conv_2"], expected)


# ---- the forward context ----


def test_forward_context_defaults_are_a_plain_pass() -> None:
    context = ForwardContext()
    assert context.decode_position is None
    assert context.attention_kernel == KERNEL_SDPA
    assert context.attention_capture == CAPTURE_NO_ATTENTION
    assert context.hook is None
    assert context.last_position_only is False
    assert context.unembed_output is True
    assert context.auxiliary_losses == []


def test_forward_contexts_do_not_share_their_loss_lists() -> None:
    first = ForwardContext()
    first.auxiliary_losses.append(torch.zeros(()))
    assert ForwardContext().auxiliary_losses == []


def test_capture_mode_names_are_the_contract_values() -> None:
    assert CAPTURE_NO_ATTENTION == "none"
    assert CAPTURE_LAST_TOKEN_ATTENTION == "last_token"
    assert CAPTURE_FULL_ATTENTION == "full"
    assert CAPTURE_MODES == ("none", "last_token", "full")


def test_unknown_capture_mode_is_refused() -> None:
    with pytest.raises(ValueError, match="unknown attention capture mode"):
        check_capture_mode("everything")


def test_decode_readout_base_class_has_no_behavior() -> None:
    readout = DecodeReadout()
    with pytest.raises(NotImplementedError):
        readout.point_hook()
    with pytest.raises(NotImplementedError):
        readout.finish_step(None, torch.zeros(1))


# ---- the call shape ----


def test_forward_context_is_keyword_only(repo_root: Path) -> None:
    model = compiled_textcnn(repo_root)
    with pytest.raises(TypeError):
        model(sample_tokens(), ForwardContext())
    with pytest.raises(TypeError):
        model.run_steps(sample_tokens(), ForwardContext())
