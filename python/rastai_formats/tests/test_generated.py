"""The generated dataclasses as a caller uses them (layout rust-control-plane.md 4.9): the shared header reads from
the JSON rastai-formats writes and writes back exactly the same keys and values, and the classes are frozen. The
core commands' argument and result types do the same, nested types and other modules' types included (M1-22).
"""

import dataclasses
import json

import pytest
from rastai_formats.generated import (
    SharedHeader,
    TrainStartArgs,
    node_status_result_from_json,
    node_status_result_to_json,
    shared_header_from_json,
    shared_header_to_json,
    train_start_args_from_json,
    train_start_args_to_json,
)

# Every header field, as rastai-formats writes it (contracts 5.1).
FULL_HEADER = {
    "$schema": "https://schemas.rastai.dev/rastai.model/1.json",
    "type": "rastai.model",
    "schema_version": 1,
    "created_with": "rastai 0.1.0",
    "id": "model_01JB3Z8Q4K7F1V0C2D9E6H5M",
    "content_hash": "blake3:" + "0" * 64,
    "derived_from": ["model_01JB3Z8Q4K7F1V0C2D9E6H5L@blake3:" + "1" * 64],
    "authors": ["Ada"],
    "license": "Apache-2.0",
    "source_url": "https://example.org/model",
    "requires": {"psychology": "^1.0.0"},
}
# A node.status result: a NodeSummary and JobSummary list from two other command modules, and the telemetry left out.
NODE_STATUS_RESULT = {
    "node": {
        "node_id": "node_01J",
        "name": "workstation",
        "transport": "local",
        "state": "connected",
        "protocol_version": 1,
        "capabilities": ["fast_runtime"],
        "last_heartbeat_unix_ms": 1760000000000,
    },
    "jobs": [{"job_id": "job_01J", "run_id": "run_01J", "node_id": "node_01J", "kind": "train", "state": "running"}],
}
# Only the four required fields.
MINIMAL_HEADER = {
    "$schema": "https://schemas.rastai.dev/rastai.lock/1.json",
    "type": "rastai.lock",
    "schema_version": 1,
    "id": "lock_1",
}


@pytest.mark.parametrize("header_json", [FULL_HEADER, MINIMAL_HEADER], ids=["full", "minimal"])
def test_the_shared_header_round_trips_through_json(header_json: dict) -> None:
    text = json.dumps(header_json)
    header = shared_header_from_json(json.loads(text))
    written = shared_header_to_json(header)
    assert written == header_json
    # The same keys in the same order, so the written file matches the one read.
    assert list(written) == list(header_json)


def test_json_keys_become_python_names() -> None:
    header = shared_header_from_json(FULL_HEADER)
    assert header.schema == FULL_HEADER["$schema"]
    assert header.type == "rastai.model"
    assert header.requires == {"psychology": "^1.0.0"}


def test_absent_optional_fields_are_none_and_stay_out() -> None:
    header = shared_header_from_json(MINIMAL_HEADER)
    assert header.license is None
    assert header.derived_from is None
    assert "license" not in shared_header_to_json(header)


def test_the_header_is_frozen_and_keyword_only() -> None:
    header = SharedHeader(schema="s", type="rastai.model", schema_version=1, id="model_1")
    with pytest.raises(dataclasses.FrozenInstanceError):
        header.id = "model_2"  # type: ignore[misc]
    with pytest.raises(TypeError):
        SharedHeader("s", "rastai.model", 1, "model_1")  # type: ignore[misc]


def test_a_missing_required_key_is_an_error() -> None:
    without_id = {key: value for key, value in MINIMAL_HEADER.items() if key != "id"}
    with pytest.raises(KeyError):
        shared_header_from_json(without_id)


def test_a_command_result_round_trips_through_its_nested_types() -> None:
    result = node_status_result_from_json(json.loads(json.dumps(NODE_STATUS_RESULT)))
    assert result.node.transport == "local"
    assert result.jobs[0].state == "running"
    assert result.telemetry is None
    assert node_status_result_to_json(result) == NODE_STATUS_RESULT


def test_command_arguments_are_keyword_only_and_optional_fields_stay_out() -> None:
    arguments = TrainStartArgs(recipe="classification", design_path="textcnn.rrsmodel", dataset_path="reviews.jsonl")
    assert arguments.eval_suite_ids is None
    written = train_start_args_to_json(arguments)
    assert written == {"recipe": "classification", "design_path": "textcnn.rrsmodel", "dataset_path": "reviews.jsonl"}
    assert train_start_args_from_json(written) == arguments
    with pytest.raises(TypeError):
        TrainStartArgs("classification", "textcnn.rrsmodel", "reviews.jsonl")  # type: ignore[misc]
