"""Event payloads: the one place where an engine's (event_type, dict) becomes protobuf (python layout 1.4 rule 7).

Engines and plugin jobs never import rastai_proto. They emit an event type from contracts 7.3 and a plain dict, and
`payload_from_dict` turns that into the typed `EventPayload` the event log and the live stream carry. It is also the
only file that imports the protobuf runtime (contracts 2.5), so the frame body's encoding and decoding live here too.

Why `encode_message` exists instead of `SerializeToString`: a frame's bytes must be the same whichever writer made
them, the Python worker or the Rust daemon (a remote run's mirror, the archive's verbatim copies, the shared fixtures).
The Rust writer encodes with prost. Python's protobuf runtime (upb) writes the same bytes except in maps, where it
differs twice: it writes a map entry's key and value even when they are the default (prost leaves them out, so a
metric of 0.0 would be 9 bytes longer), and its deterministic key order puts "ab" before "a" (prost writes keys in
byte order, "a" before "ab"). `encode_message` writes prost's form: fields in number order, a field left out when
it holds its default and has no presence, numeric repeated fields packed, and map entries in key order with a
default key or value left out. The messages in proto/ declare their fields in number order with any oneof last
(contracts 4.3), which is the order prost writes them in; tests/test_event_payloads.py checks that rule.
"""

import struct

from google.protobuf import json_format
from google.protobuf.descriptor import FieldDescriptor
from google.protobuf.message import DecodeError, Message
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.events.v1.payloads_pb2 import EventPayload

# The event types a worker writes to its log or streams live (contracts 7.3, emitter "worker"), and the field of the
# EventPayload oneof that holds each one's payload. The other M1 types come from the node or the daemon, so a worker
# that tried to emit one is a bug.
WORKER_EVENT_PAYLOAD_FIELDS = {
    "job.progress": "job_progress",
    "run.state_changed": "run_state_changed",
    "run.metrics": "run_metrics",
    "checkpoint.saved": "checkpoint_saved",
    "artifact.created": "artifact_created",
    "diagnostic.reported": "diagnostic_reported",
}

# The protobuf wire types: the low 3 bits of a field key.
WIRE_TYPE_VARINT = 0
WIRE_TYPE_FIXED64 = 1
WIRE_TYPE_LENGTH_DELIMITED = 2
WIRE_TYPE_FIXED32 = 5
WIRE_TYPE_BITS = 3
# Varints carry unsigned 64-bit values; a negative int32, int64 or enum is written as its 64-bit two's complement.
UINT64_MASK = (1 << 64) - 1
UINT32_MASK = (1 << 32) - 1

VARINT_TYPES = {
    FieldDescriptor.TYPE_INT32,
    FieldDescriptor.TYPE_INT64,
    FieldDescriptor.TYPE_UINT32,
    FieldDescriptor.TYPE_UINT64,
    FieldDescriptor.TYPE_BOOL,
    FieldDescriptor.TYPE_ENUM,
}
# Zigzag types, and the width of each: sint32 is zigzagged as 32 bits, sint64 as 64.
ZIGZAG_BITS = {FieldDescriptor.TYPE_SINT32: 32, FieldDescriptor.TYPE_SINT64: 64}
# Little-endian fixed-width types and their struct formats.
FIXED64_FORMATS = {
    FieldDescriptor.TYPE_DOUBLE: "<d",
    FieldDescriptor.TYPE_FIXED64: "<Q",
    FieldDescriptor.TYPE_SFIXED64: "<q",
}
FIXED32_FORMATS = {
    FieldDescriptor.TYPE_FLOAT: "<f",
    FieldDescriptor.TYPE_FIXED32: "<I",
    FieldDescriptor.TYPE_SFIXED32: "<i",
}
LENGTH_DELIMITED_TYPES = {FieldDescriptor.TYPE_STRING, FieldDescriptor.TYPE_BYTES, FieldDescriptor.TYPE_MESSAGE}
# A map entry's two fields (protobuf's map encoding: a repeated message with key = 1 and value = 2).
MAP_KEY_FIELD = "key"
MAP_VALUE_FIELD = "value"


# ---- From an engine's dict ----


def payload_from_dict(event_type: str, values: dict) -> EventPayload:
    """The typed payload of one event: `values` holds the payload message's fields by their proto names, enums by
    name ("RUN_STATE_RUNNING"). An event type a worker doesn't emit, a field the message doesn't have or a value
    that can't become the field's type is a bug in the calling code, so it raises ValueError.

    The conversion is protobuf's JSON mapping, so it also takes what that mapping takes: a field's lowerCamelCase
    JSON name ("checkpointId" for checkpoint_id), a number written as a string ("12" for an int64) and an enum by
    its number. Engines write proto names and plain values; the record is the same either way."""
    field_name = WORKER_EVENT_PAYLOAD_FIELDS.get(event_type)
    if field_name is None:
        raise ValueError(f"a worker doesn't emit the event type {event_type!r} (contracts 7.3)")
    payload = EventPayload()
    message = getattr(payload, field_name)
    try:
        # ignore_unknown_fields stays False: a misspelled field must fail here, not vanish from the record.
        json_format.ParseDict(values, message)
    except json_format.ParseError as error:
        raise ValueError(f"the {event_type} payload doesn't match its message: {error}") from error
    # An empty dict sets no field, so mark the oneof's choice explicitly: the payload still says which event it is.
    message.SetInParent()
    return payload


# ---- Frame bodies ----


def decode_event_record(body: bytes) -> EventRecord | None:
    """The record in a frame body, or None when the bytes aren't an EventRecord."""
    try:
        return EventRecord.FromString(body)
    except DecodeError:
        return None


def encode_message(message: Message) -> bytes:
    """`message` in prost's encoding (see the top of this file), byte for byte what the Rust writer writes."""
    parts = []
    for field, value in message.ListFields():
        if is_left_out(field, value):
            continue
        parts.append(encode_field(field, value))
    return b"".join(parts)


def is_left_out(field: FieldDescriptor, value: object) -> bool:
    """True for a field prost leaves out that ListFields still lists: a field without presence holding -0.0, which
    upb counts as set (its bits aren't zero) and prost counts as its default (-0.0 == 0.0)."""
    if field.has_presence or field.is_repeated:
        return False
    return value == field.default_value


def encode_field(field: FieldDescriptor, value: object) -> bytes:
    if is_map_field(field):
        return encode_map(field, value)
    if field.is_repeated:
        return encode_repeated(field, value)
    return encode_single(field, value)


def is_map_field(field: FieldDescriptor) -> bool:
    return field.type == FieldDescriptor.TYPE_MESSAGE and field.message_type.GetOptions().map_entry


def encode_map(field: FieldDescriptor, entries: object) -> bytes:
    """One length-delimited entry per key, in key order. Python orders text by code point, which for UTF-8 is the
    byte order prost's BTreeMap uses; numbers and booleans order the same in both."""
    key_field = field.message_type.fields_by_name[MAP_KEY_FIELD]
    value_field = field.message_type.fields_by_name[MAP_VALUE_FIELD]
    parts = []
    for key in sorted(entries):
        entry = encode_map_entry_field(key_field, key) + encode_map_entry_field(value_field, entries[key])
        parts.append(encode_key(field.number, WIRE_TYPE_LENGTH_DELIMITED) + encode_varint(len(entry)) + entry)
    return b"".join(parts)


def encode_map_entry_field(field: FieldDescriptor, value: object) -> bytes:
    """A map entry's key or value, left out when it is the default (an empty message counts as default too)."""
    if field.type == FieldDescriptor.TYPE_MESSAGE:
        if not value.ListFields():
            return b""
    elif value == field.default_value:
        return b""
    return encode_single(field, value)


def encode_repeated(field: FieldDescriptor, values: object) -> bytes:
    """Text, bytes and messages are written once per item; numbers, booleans and enums are packed into one
    length-delimited field, the proto3 default that prost follows."""
    if field.type in LENGTH_DELIMITED_TYPES:
        return b"".join(encode_single(field, value) for value in values)
    packed = b"".join(encode_scalar(field, value) for value in values)
    return encode_key(field.number, WIRE_TYPE_LENGTH_DELIMITED) + encode_varint(len(packed)) + packed


def encode_single(field: FieldDescriptor, value: object) -> bytes:
    """One field: its key, then its value (with a length in front for text, bytes and messages)."""
    if field.type in LENGTH_DELIMITED_TYPES:
        data = encode_length_delimited_value(field, value)
        return encode_key(field.number, WIRE_TYPE_LENGTH_DELIMITED) + encode_varint(len(data)) + data
    return encode_key(field.number, scalar_wire_type(field)) + encode_scalar(field, value)


def encode_length_delimited_value(field: FieldDescriptor, value: object) -> bytes:
    if field.type == FieldDescriptor.TYPE_MESSAGE:
        return encode_message(value)
    if field.type == FieldDescriptor.TYPE_STRING:
        return value.encode("utf-8")
    return bytes(value)


def scalar_wire_type(field: FieldDescriptor) -> int:
    if field.type in FIXED64_FORMATS:
        return WIRE_TYPE_FIXED64
    if field.type in FIXED32_FORMATS:
        return WIRE_TYPE_FIXED32
    return WIRE_TYPE_VARINT


def encode_scalar(field: FieldDescriptor, value: object) -> bytes:
    """A number, boolean or enum value without its key, as both a single field and a packed item write it."""
    if field.type in FIXED64_FORMATS:
        return struct.pack(FIXED64_FORMATS[field.type], value)
    if field.type in FIXED32_FORMATS:
        return struct.pack(FIXED32_FORMATS[field.type], value)
    if field.type in ZIGZAG_BITS:
        return encode_varint(zigzag(value, ZIGZAG_BITS[field.type]))
    if field.type in VARINT_TYPES:
        return encode_varint(int(value) & UINT64_MASK)
    # Groups are proto2 only and never in proto/.
    raise ValueError(f"field {field.full_name} has a type the event encoder doesn't write: {field.type}")


def zigzag(value: int, bits: int) -> int:
    """Signed to unsigned so small negative numbers stay short: 0, -1, 1, -2 become 0, 1, 2, 3."""
    mask = UINT32_MASK if bits == 32 else UINT64_MASK
    return ((value << 1) ^ (value >> (bits - 1))) & mask


def encode_key(field_number: int, wire_type: int) -> bytes:
    return encode_varint((field_number << WIRE_TYPE_BITS) | wire_type)


def encode_varint(value: int) -> bytes:
    """Seven bits per byte, low bits first; the top bit of each byte says another byte follows."""
    output = bytearray()
    while value >= 0x80:
        output.append((value & 0x7F) | 0x80)
        value >>= 7
    output.append(value)
    return bytes(output)
