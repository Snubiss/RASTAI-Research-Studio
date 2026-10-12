"""The compiler (layout python.md section 2.7): the TextCNN template compiles to the parameter counts stored in
`fixtures/models/textcnn/expected_counts.json`, seeded init repeats exactly, a strict fill refuses a missing, extra
or mis-shaped tensor before copying anything, train modes freeze what they say, and the graph runs in order.

Every spec here goes through `rastai_formats.load_model_spec` first, the one validation, as a worker's does.
"""

import copy
import json
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.model import ModelSpec, model_spec_from_json
from rastai_models.compiler.graph_order import order_graph
from rastai_plugin.catalog.generated import RRS_M0001, RRS_M2002, RRS_M2003, RRS_M2004

import rastai_formats
from rastai_models import (
    CompiledModel,
    ModelError,
    SafetensorsFile,
    bind_weights,
    compile_model,
    init_weights,
    write_safetensors,
)

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
TEXTCNN_COUNTS = Path("fixtures/models/textcnn/expected_counts.json")
SEED = 35
OTHER_SEED = 36
BATCH_SIZE = 2


def read_spec(json_bytes: bytes) -> ModelSpec:
    """A ModelSpec the way a worker gets one: validated by the wheel, then read into the generated dataclass."""
    return model_spec_from_json(json.loads(rastai_formats.load_model_spec(json_bytes)))


def template_json(repo_root: Path) -> dict:
    return json.loads((repo_root / TEXTCNN_TEMPLATE).read_text(encoding="utf-8"))


def compile_template(repo_root: Path) -> CompiledModel:
    spec = read_spec((repo_root / TEXTCNN_TEMPLATE).read_bytes())
    return compile_model(spec, "cpu", torch.float32)


def compile_document(document: dict) -> CompiledModel:
    spec = read_spec(json.dumps(document).encode("utf-8"))
    return compile_model(spec, "cpu", torch.float32)


def node_named(document: dict, node_id: str) -> dict:
    return next(node for node in document["graph"]["nodes"] if node["node_id"] == node_id)


def random_tokens(vocab_size: int, length: int) -> torch.Tensor:
    return torch.randint(0, vocab_size, (BATCH_SIZE, length), generator=torch.Generator().manual_seed(SEED))


def write_graph_parameters(model: CompiledModel, path: Path) -> dict[str, torch.Tensor]:
    """Writes every parameter under its graph path, as a checkpoint would, and returns what it wrote."""
    tensors = {name: parameter.detach().clone() for name, parameter in model.graph_parameters().items()}
    write_safetensors(path, tensors)
    return tensors


# ---- the TextCNN template ----


def test_textcnn_parameter_counts_equal_the_stored_counts(repo_root: Path) -> None:
    expected = json.loads((repo_root / TEXTCNN_COUNTS).read_text(encoding="utf-8"))
    assert expected["design"] == TEXTCNN_TEMPLATE.as_posix()
    model = compile_template(repo_root)

    assert sum(parameter.numel() for parameter in model.parameters()) == expected["parameter_count"]
    trainable = sum(parameter.numel() for parameter in model.parameters() if parameter.requires_grad)
    assert trainable == expected["trainable_parameter_count"]
    node_counts = {step.node.node_id: 0 for step in model.graph_steps}
    for name, parameter in model.graph_parameters().items():
        node_counts[name.rsplit(".", 1)[0]] += parameter.numel()
    assert node_counts == expected["node_parameter_counts"]


def test_textcnn_runs_tokens_to_class_logits(repo_root: Path) -> None:
    model = compile_template(repo_root)
    init_weights(model, SEED)
    model.eval()
    logits = model(random_tokens(vocab_size=25_000, length=128))
    assert logits.shape == (BATCH_SIZE, 4)
    assert torch.isfinite(logits).all()


def test_textcnn_trains_end_to_end(repo_root: Path) -> None:
    """Every parameter gets a gradient: the graph is wired from the embedding through to the head."""
    model = compile_template(repo_root)
    init_weights(model, SEED)
    model.train()
    loss = torch.nn.functional.cross_entropy(model(random_tokens(25_000, 128)), torch.tensor([0, 3]))
    loss.backward()
    without_gradient = [name for name, parameter in model.graph_parameters().items() if parameter.grad is None]
    assert without_gradient == []


def test_dropout_acts_only_in_train_mode(repo_root: Path) -> None:
    model = compile_template(repo_root)
    init_weights(model, SEED)
    tokens = random_tokens(25_000, 128)
    model.eval()
    assert torch.equal(model(tokens), model(tokens))
    model.train()
    torch.manual_seed(SEED)
    first = model(tokens)
    torch.manual_seed(OTHER_SEED)
    assert not torch.equal(first, model(tokens))


def test_parameters_are_named_by_graph_path(repo_root: Path) -> None:
    names = list(compile_template(repo_root).graph_parameters())
    assert names[:4] == ["embedding.weight", "conv_1.weight", "conv_1.bias", "conv_2.weight"]
    assert names[-2:] == ["classifier.weight", "classifier.bias"]


# ---- seeded init ----


def test_seeded_init_repeats_exactly(repo_root: Path) -> None:
    first = compile_template(repo_root)
    second = compile_template(repo_root)
    init_weights(first, SEED)
    init_weights(second, SEED)
    for name, parameter in first.graph_parameters().items():
        assert torch.equal(parameter, second.graph_parameters()[name]), name


def test_another_seed_gives_other_weights(repo_root: Path) -> None:
    first = compile_template(repo_root)
    second = compile_template(repo_root)
    init_weights(first, SEED)
    init_weights(second, OTHER_SEED)
    assert not torch.equal(first.graph_parameters()["conv_1.weight"], second.graph_parameters()["conv_1.weight"])


def test_seeded_init_gives_the_same_values_in_another_dtype(repo_root: Path) -> None:
    """Values are drawn in float32 on the CPU and then cast, so a bf16 model holds the float32 model's values
    rounded, not a different draw."""
    spec = read_spec((repo_root / TEXTCNN_TEMPLATE).read_bytes())
    full = compile_model(spec, "cpu", torch.float32)
    half = compile_model(spec, "cpu", torch.bfloat16)
    init_weights(full, SEED)
    init_weights(half, SEED)
    for name, parameter in full.graph_parameters().items():
        assert torch.equal(parameter.to(torch.bfloat16), half.graph_parameters()[name]), name


def test_init_spreads_like_torch_defaults(repo_root: Path) -> None:
    model = compile_template(repo_root)
    init_weights(model, SEED)
    parameters = model.graph_parameters()
    # conv_1: fan_in = 128 channels × kernel 3, so every value lies within ±1/sqrt(384).
    assert parameters["conv_1.weight"].abs().max() <= 1 / (384**0.5)
    # The embedding is standard normal: 3.2 million draws have a spread close to 1.
    assert abs(parameters["embedding.weight"].std().item() - 1.0) < 0.01


# ---- strict weight fill ----


def test_strict_fill_copies_every_tensor(repo_root: Path, tmp_path: Path) -> None:
    source = compile_template(repo_root)
    init_weights(source, SEED)
    written = write_graph_parameters(source, tmp_path / "model.safetensors")

    target = compile_template(repo_root)
    with SafetensorsFile(tmp_path / "model.safetensors") as weights:
        report = bind_weights(target, [weights])

    assert report.parameter_names == list(written)
    assert report.value_count == sum(tensor.numel() for tensor in written.values())
    for name, parameter in target.graph_parameters().items():
        assert torch.equal(parameter, written[name]), name


def test_strict_fill_reads_tensors_spread_over_two_files(repo_root: Path, tmp_path: Path) -> None:
    source = compile_template(repo_root)
    init_weights(source, SEED)
    tensors = {name: parameter.detach() for name, parameter in source.graph_parameters().items()}
    embedding_names = {"embedding.weight"}
    write_safetensors(tmp_path / "a.safetensors", {name: tensors[name] for name in embedding_names})
    write_safetensors(tmp_path / "b.safetensors", {n: t for n, t in tensors.items() if n not in embedding_names})

    target = compile_template(repo_root)
    with SafetensorsFile(tmp_path / "a.safetensors") as first, SafetensorsFile(tmp_path / "b.safetensors") as second:
        bind_weights(target, [first, second])
    assert torch.equal(target.graph_parameters()["embedding.weight"], tensors["embedding.weight"])


def refused_bind(repo_root: Path, tmp_path: Path, tensors: dict[str, torch.Tensor]) -> tuple[ModelError, dict]:
    """Binds `tensors` into a fresh template model that must refuse them; returns the error and the model's
    parameters after the refusal."""
    write_safetensors(tmp_path / "bad.safetensors", tensors)
    target = compile_template(repo_root)
    init_weights(target, OTHER_SEED)
    before = {name: parameter.detach().clone() for name, parameter in target.graph_parameters().items()}
    with SafetensorsFile(tmp_path / "bad.safetensors") as weights, pytest.raises(ModelError) as refusal:
        bind_weights(target, [weights])
    for name, parameter in target.graph_parameters().items():
        assert torch.equal(parameter, before[name]), f"{name} changed although the bind was refused"
    return refusal.value, before


def full_tensors(repo_root: Path) -> dict[str, torch.Tensor]:
    source = compile_template(repo_root)
    init_weights(source, SEED)
    return {name: parameter.detach().clone() for name, parameter in source.graph_parameters().items()}


def test_a_missing_tensor_is_refused_before_any_copy(repo_root: Path, tmp_path: Path) -> None:
    tensors = full_tensors(repo_root)
    del tensors["conv_3.bias"]
    error, _ = refused_bind(repo_root, tmp_path, tensors)
    assert error.code == RRS_M2002
    assert "conv_3.bias" in error.message


def test_an_extra_tensor_is_refused_before_any_copy(repo_root: Path, tmp_path: Path) -> None:
    tensors = full_tensors(repo_root)
    tensors["conv_6.weight"] = torch.zeros(3)
    error, _ = refused_bind(repo_root, tmp_path, tensors)
    assert error.code == RRS_M2003
    assert "conv_6.weight" in error.message


def test_a_mis_shaped_tensor_is_refused_before_any_copy(repo_root: Path, tmp_path: Path) -> None:
    tensors = full_tensors(repo_root)
    tensors["classifier.weight"] = torch.zeros(5, 64)
    error, _ = refused_bind(repo_root, tmp_path, tensors)
    assert error.code == RRS_M2004
    assert error.message == "Tensor classifier.weight has shape [5, 64], but the ModelSpec needs [4, 64]."


def test_a_tensor_in_two_files_is_refused(repo_root: Path, tmp_path: Path) -> None:
    tensors = full_tensors(repo_root)
    write_safetensors(tmp_path / "a.safetensors", tensors)
    write_safetensors(tmp_path / "b.safetensors", {"conv_1.bias": tensors["conv_1.bias"]})
    target = compile_template(repo_root)
    with SafetensorsFile(tmp_path / "a.safetensors") as first, SafetensorsFile(tmp_path / "b.safetensors") as second:
        with pytest.raises(ModelError) as refusal:
            bind_weights(target, [first, second])
    assert refusal.value.code == RRS_M2003
    assert "conv_1.bias" in refusal.value.message


def test_a_long_list_of_names_is_cut_short(repo_root: Path, tmp_path: Path) -> None:
    tensors = full_tensors(repo_root)
    for index in range(12):
        tensors[f"unused_{index:02}"] = torch.zeros(1)
    error, _ = refused_bind(repo_root, tmp_path, tensors)
    assert error.message.endswith("unused_09 and 2 more.")


# ---- train modes ----


def test_freeze_and_lora_nodes_get_no_gradients(repo_root: Path) -> None:
    document = template_json(repo_root)
    node_named(document, "embedding")["train_mode"] = "freeze"
    node_named(document, "conv_1")["train_mode"] = "lora"
    model = compile_document(document)
    frozen = [name for name, parameter in model.graph_parameters().items() if not parameter.requires_grad]
    assert frozen == ["embedding.weight", "conv_1.weight", "conv_1.bias"]


def test_a_left_out_train_mode_trains(repo_root: Path) -> None:
    document = template_json(repo_root)
    del node_named(document, "classifier")["train_mode"]
    model = compile_document(document)
    assert all(parameter.requires_grad for parameter in model.parameters())


# ---- graph order and the op table ----


def concat_document(repo_root: Path) -> dict:
    """The template's embedding feeding two linear branches joined by a concat, with the nodes listed so that the
    concat and a branch come before what feeds them."""
    document = template_json(repo_root)
    embedding = copy.deepcopy(node_named(document, "embedding"))
    embedding["attributes"] = {"vocab_size": 10, "embedding_size": 4}
    branch_a = {"node_id": "branch_a", "op": "linear", "attributes": {"in_features": 4, "out_features": 3}}
    branch_b = {"node_id": "branch_b", "op": "linear", "attributes": {"in_features": 4, "out_features": 5}}
    join = {"node_id": "join", "op": "concat", "attributes": {}}
    document["graph"]["nodes"] = [join, branch_b, embedding, branch_a]
    document["graph"]["edges"] = [
        {"from_node": "branch_b", "to_node": "join", "to_input": 1},
        {"from_node": "branch_a", "to_node": "join", "to_input": 0},
        {"from_node": "embedding", "to_node": "branch_a"},
        {"from_node": "embedding", "to_node": "branch_b"},
    ]
    document["inputs"][0]["range"] = {"start": 0, "end": 6}
    document["outputs"] = [{"name": "joined", "graph_node": "join"}]
    return document


def test_nodes_run_after_what_feeds_them_with_ties_in_file_order(repo_root: Path) -> None:
    spec = read_spec(json.dumps(concat_document(repo_root)).encode("utf-8"))
    order = [step.node.node_id for step in order_graph(spec)]
    assert order == ["embedding", "branch_b", "branch_a", "join"]


def test_concat_inputs_are_taken_in_to_input_order(repo_root: Path) -> None:
    model = compile_document(concat_document(repo_root))
    init_weights(model, SEED)
    tokens = random_tokens(vocab_size=10, length=8)
    outputs = model.run_steps(tokens)
    joined = model(tokens)
    # Input 0 (branch_a, 3 features) comes first although its edge is listed second.
    assert joined.shape == (BATCH_SIZE, 6, 8)
    assert torch.equal(joined[..., :3], outputs["branch_a"])
    assert torch.equal(joined[..., 3:], outputs["branch_b"])


def test_an_input_map_range_takes_its_positions(repo_root: Path) -> None:
    """The template's range is moved to positions 2 to 6: the embedding sees only those."""
    document = concat_document(repo_root)
    document["inputs"][0]["range"] = {"start": 2, "end": 6}
    model = compile_document(document)
    init_weights(model, SEED)
    tokens = random_tokens(vocab_size=10, length=8)
    embedded = model.run_steps(tokens)["embedding"]
    assert torch.equal(embedded, model.graph_parameters()["embedding.weight"][tokens[:, 2:6]])


def test_an_op_without_an_implementation_is_rrs_m0001(repo_root: Path) -> None:
    document = template_json(repo_root)
    node_named(document, "dropout")["op"] = "rms_norm"
    node_named(document, "dropout")["attributes"] = {}
    with pytest.raises(ModelError) as refusal:
        compile_document(document)
    assert refusal.value.code == RRS_M0001
    assert refusal.value.message.startswith("The graph node dropout (rms_norm) has no implementation")
