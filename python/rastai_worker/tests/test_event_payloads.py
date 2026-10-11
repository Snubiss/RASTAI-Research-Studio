"""event_payloads.py: an engine's (event_type, dict) to the typed payload, and the frame body's encoding.

The encoder must write prost's bytes (the top of event_payloads.py says why). Three tests hold it to that: the proto
goldens, whose .binpb files crates/rastai-proto/tests/golden.rs checks against prost, the map cases where Python's
own serializer differs from prost, written out byte by byte, and fixtures/event-logs/from_python.log, which
test_event_log.py writes and crates/rastai-store/tests/python_log_compat.rs reads.
"""

from pathlib import Path

import pytest
from rastai_proto.common.v1 import blob_pb2, diagnostic_pb2, hardware_pb2, states_pb2
from rastai_proto.events.v1 import event_record_pb2, payloads_pb2
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.events.v1.payloads_pb2 import CheckpointSaved, EventPayload, RunMetrics, RunStateChanged
from rastai_worker.event_payloads import (
    WORKER_EVENT_PAYLOAD_FIELDS,
    decode_event_record,
    encode_message,
    payload_from_dict,
)

GOLDEN_EVENTS_FOLDER = Path("proto-golden/events")
GOLDEN_COMMON_FOLDER = Path("proto-golden/common")
BINARY_SUFFIX = ".binpb"
# The generated modules whose messages a frame body can hold: the record, its payloads and what they carry.
EVENT_RECORD_MODULES = [event_record_pb2, payloads_pb2, blob_pb2, diagnostic_pb2, hardware_pb2, states_pb2]
# A filled payload per event type a worker emits, in the form engines write them: proto field names, enums by name.
SAMPLE_PAYLOADS = {
    "job.progress": {"fraction": 0.25, "message": "tokenizing 3 of 8 files"},
    "run.state_changed": {"reason": "stopped", "run_state": "RUN_STATE_COMPLETED"},
    "run.metrics": {"step": 12, "values": {"train.loss": 2.31, "train.learning_rate": 0.0003}},
    "checkpoint.saved": {
        "checkpoint_id": "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2T",
        "step": 12,
        "folder": "checkpoints/step_00000012",
        "file_hashes": {"model.safetensors": "blake3:" + "ab" * 32},
        "is_best": True,
    },
    "artifact.created": {
        "artifact_id": "art_01JA2B3C4D5E6F7G8H9J0K1M2V",
        "kind": "checkpoint_export",
        "content_hash": "blake3:" + "cd" * 32,
        "size_bytes": 4096,
        "home_node_id": "node_local",
        "parents": [{"id": "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2T", "relation": "exported_as"}],
    },
    "diagnostic.reported": {
        "diagnostic": {"code": "RRS-S2001", "severity": "SEVERITY_INFO", "message": "torn tail cut"},
        "subject_id": "run_01JA2B3C4D5E6F7G8H9J0K1M2S",
    },
}


# ---- payload_from_dict ----


def test_every_worker_event_type_has_a_sample() -> None:
    # Guards the parametrized tests below: a new worker event type must bring its sample.
    assert sorted(SAMPLE_PAYLOADS) == sorted(WORKER_EVENT_PAYLOAD_FIELDS)


@pytest.mark.parametrize("event_type", sorted(SAMPLE_PAYLOADS))
def test_every_worker_event_type_converts_from_its_dict(event_type: str) -> None:
    payload = payload_from_dict(event_type, SAMPLE_PAYLOADS[event_type])
    field_name = WORKER_EVENT_PAYLOAD_FIELDS[event_type]
    assert payload.WhichOneof("payload") == field_name
    # The field name is the event type with its dot as an underscore (payloads.proto).
    assert field_name == event_type.replace(".", "_")


@pytest.mark.parametrize("event_type", sorted(SAMPLE_PAYLOADS))
def test_an_unknown_field_fails(event_type: str) -> None:
    values = dict(SAMPLE_PAYLOADS[event_type])
    values["not_a_field"] = 1
    with pytest.raises(ValueError, match=event_type):
        payload_from_dict(event_type, values)


def test_the_values_reach_the_typed_message() -> None:
    payload = payload_from_dict("checkpoint.saved", SAMPLE_PAYLOADS["checkpoint.saved"])
    saved = payload.checkpoint_saved
    assert saved.step == 12
    assert saved.is_best
    assert dict(saved.file_hashes) == {"model.safetensors": "blake3:" + "ab" * 32}
    state = payload_from_dict("run.state_changed", SAMPLE_PAYLOADS["run.state_changed"]).run_state_changed
    assert state.WhichOneof("state") == "run_state"
    assert state.run_state == states_pb2.RUN_STATE_COMPLETED


def test_an_empty_dict_still_says_which_event_it_is() -> None:
    payload = payload_from_dict("run.state_changed", {})
    assert payload.WhichOneof("payload") == "run_state_changed"


def test_an_event_type_a_worker_does_not_emit_fails() -> None:
    with pytest.raises(ValueError, match="node.state_changed"):
        payload_from_dict("node.state_changed", {})
    with pytest.raises(ValueError, match="run.metric"):
        payload_from_dict("run.metric", {"step": 1})


def test_a_value_of_the_wrong_type_fails() -> None:
    with pytest.raises(ValueError, match="run.metrics"):
        payload_from_dict("run.metrics", {"step": "twelve"})
    with pytest.raises(ValueError, match="run.state_changed"):
        payload_from_dict("run.state_changed", {"run_state": "RUN_STATE_SLEEPING"})


# ---- encode_message: prost's bytes ----


def golden_cases(fixtures_folder: Path) -> list[tuple[str, type, bytes]]:
    """(name, class, golden bytes) for every frame-body message that has a golden in events/ or common/."""
    cases = []
    for module in EVENT_RECORD_MODULES:
        for name in sorted(module.DESCRIPTOR.message_types_by_name):
            for folder in (GOLDEN_EVENTS_FOLDER, GOLDEN_COMMON_FOLDER):
                golden_path = fixtures_folder / folder / f"{name}{BINARY_SUFFIX}"
                if golden_path.is_file():
                    cases.append((name, getattr(module, name), golden_path.read_bytes()))
    return cases


def test_every_frame_body_golden_encodes_to_its_prost_bytes(fixtures_folder: Path) -> None:
    cases = golden_cases(fixtures_folder)
    # EventRecord, its payloads and the common messages they carry: a broken path would find none.
    assert len(cases) > 20
    for name, message_class, golden_bytes in cases:
        message = message_class.FromString(golden_bytes)
        assert encode_message(message) == golden_bytes, f"{name} differs from its golden"


def test_map_entries_are_in_byte_order_with_defaults_left_out() -> None:
    metrics = RunMetrics(step=1)
    metrics.values["ab"] = 0.0
    metrics.values["b"] = 3.0
    metrics.values["a"] = 1.0
    metrics.values[""] = 2.0
    # prost's form: step, then the entries "", "a", "ab", "b" in byte order. An entry leaves out a default key ("")
    # or value (0.0); Python's own deterministic serializer writes both and puts "ab" before "a".
    expected = bytes.fromhex(
        "08 01"  # step = 1
        " 12 09 11 0000000000000040"  # "" -> 2.0: the key is left out
        " 12 0c 0a 01 61 11 000000000000f03f"  # "a" -> 1.0
        " 12 04 0a 02 6162"  # "ab" -> 0.0: the value is left out
        " 12 0c 0a 01 62 11 0000000000000840"  # "b" -> 3.0
    )
    assert encode_message(metrics) == expected
    assert RunMetrics.FromString(expected) == metrics


def test_a_string_map_leaves_out_an_empty_value() -> None:
    saved = CheckpointSaved(step=2, is_best=True)
    saved.file_hashes["model.safetensors"] = ""
    # step = 2, then the entry with its key only (field 4, 19 bytes: key field 1, 17 bytes of text), then is_best.
    expected = bytes.fromhex("10 02 22 13 0a 11") + b"model.safetensors" + bytes.fromhex("28 01")
    assert encode_message(saved) == expected


def test_a_negative_zero_without_presence_is_left_out_like_prost() -> None:
    progress = payloads_pb2.JobProgress(fraction=-0.0, message="x")
    assert encode_message(progress) == bytes.fromhex("12 01") + b"x"


def test_a_chosen_oneof_member_is_written_even_when_zero() -> None:
    # A oneof member has presence: RUN_STATE_UNSPECIFIED (0) chosen is still written, as prost writes Some(0).
    state = RunStateChanged(run_state=states_pb2.RUN_STATE_UNSPECIFIED)
    assert encode_message(state) == bytes.fromhex("1000")


def test_an_encoded_record_decodes_to_the_same_record() -> None:
    record = EventRecord(
        format_version=1,
        sequence=7,
        run_id="run_01JA2B3C4D5E6F7G8H9J0K1M2S",
        event_type="artifact.created",
        payload=payload_from_dict("artifact.created", SAMPLE_PAYLOADS["artifact.created"]),
        blobs=[blob_pb2.BlobRef(hash="blake3:" + "cd" * 32)],
    )
    assert decode_event_record(encode_message(record)) == record


def test_bytes_that_are_not_a_record_decode_to_none() -> None:
    assert decode_event_record(b"\xff\xff\xff") is None


# ---- The order rule encode_message relies on ----


def message_types_reachable_from(descriptor: object, seen: dict[str, object]) -> None:
    if descriptor.full_name in seen:
        return
    seen[descriptor.full_name] = descriptor
    for field in descriptor.fields:
        if field.message_type is not None:
            message_types_reachable_from(field.message_type, seen)


def is_real_oneof_member(field: object) -> bool:
    """A member of a declared oneof, not of the hidden one protobuf makes for a proto3 `optional` field."""
    oneof = field.containing_oneof
    return oneof is not None and oneof.name != "_" + field.name


def test_every_frame_body_message_declares_fields_in_number_order_with_oneofs_last() -> None:
    # prost writes fields in declaration order with a oneof after the rest; encode_message writes them in number
    # order. The two agree only while proto/ keeps this rule (contracts 4.3), so a .proto that breaks it fails here.
    reachable: dict[str, object] = {}
    message_types_reachable_from(EventRecord.DESCRIPTOR, reachable)
    assert "rastai.events.v1.RunMetrics" in reachable
    for full_name, descriptor in reachable.items():
        numbers = [field.number for field in descriptor.fields]
        assert numbers == sorted(numbers), f"{full_name} declares its fields out of number order"
        plain_numbers = [field.number for field in descriptor.fields if not is_real_oneof_member(field)]
        oneof_numbers = [field.number for field in descriptor.fields if is_real_oneof_member(field)]
        if plain_numbers and oneof_numbers:
            assert max(plain_numbers) < min(oneof_numbers), f"{full_name} has a field after its oneof"


def test_event_payload_is_one_oneof_over_every_payload() -> None:
    oneof = EventPayload.DESCRIPTOR.oneofs_by_name["payload"]
    assert len(oneof.fields) == len(EventPayload.DESCRIPTOR.fields)
