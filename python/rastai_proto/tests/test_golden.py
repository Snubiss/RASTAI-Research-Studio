"""Golden messages (layout: rust-node-gateway-proto.md section 3.5).

Every message of rastai.common.v1, rastai.commands.v1 and rastai.events.v1 has a golden in
fixtures/proto-golden/<package>/, and its .binpb decodes and encodes back to exactly the same bytes. The Rust and
JavaScript tests compare against the same files, so the three encoders agree byte for byte. Python keeps fields it
doesn't know and writes them back, which would hide a module that lags behind the .proto, so the test drops unknown
fields before encoding.
"""

import importlib
import pkgutil
from pathlib import Path
from types import ModuleType

import pytest
import rastai_proto.commands.v1
import rastai_proto.common.v1
import rastai_proto.events.v1
from rastai_proto.commands.v1.command_service_pb2 import CommandDescriptor
from rastai_proto.common.v1.states_pb2 import STABILITY_PREVIEW, STABILITY_UNSPECIFIED
from rastai_proto.events.v1.event_service_pb2 import EventTypeDescriptor

GOLDEN_ROOT = Path("proto-golden")
BINARY_SUFFIX = ".binpb"
JSON_SUFFIX = ".json"
# Generated message modules end in _pb2; the _pb2_grpc modules beside them hold service stubs only.
MESSAGE_MODULE_SUFFIX = "_pb2"
# Descriptors written by a daemon that predates the stability field (layout 3.5, "Unset stability").
WITHOUT_STABILITY_FOLDER = "without_stability"
# Golden folder name -> the generated package whose messages it holds.
GOLDEN_PACKAGES = {
    "common": rastai_proto.common.v1,
    "commands": rastai_proto.commands.v1,
    "events": rastai_proto.events.v1,
}


def message_modules(package: ModuleType) -> list[ModuleType]:
    """Every generated message module of a package, found on disk so a new .proto is never missed."""
    modules = []
    for module_info in pkgutil.iter_modules(package.__path__):
        if module_info.name.endswith(MESSAGE_MODULE_SUFFIX):
            modules.append(importlib.import_module(f"{package.__name__}.{module_info.name}"))
    return modules


def message_classes(package: ModuleType) -> dict[str, type]:
    """Message name -> generated class, for every top-level message of a package."""
    classes = {}
    for module in message_modules(package):
        for name in module.DESCRIPTOR.message_types_by_name:
            classes[name] = getattr(module, name)
    return classes


MESSAGE_CLASSES = {folder: message_classes(package) for folder, package in GOLDEN_PACKAGES.items()}
# (golden folder, message name) for every message of every package, so each gets its own test case.
ALL_MESSAGES = [(folder, name) for folder, classes in MESSAGE_CLASSES.items() for name in sorted(classes)]


def read_golden_bytes(fixtures_folder: Path, folder: str, message_name: str) -> bytes:
    golden_path = fixtures_folder / GOLDEN_ROOT / folder / f"{message_name}{BINARY_SUFFIX}"
    assert golden_path.is_file(), f"{message_name} has no golden; add {golden_path.name} (fixtures/proto-golden/)"
    return golden_path.read_bytes()


@pytest.mark.parametrize("folder", sorted(GOLDEN_PACKAGES))
def test_every_package_has_messages(folder: str) -> None:
    # Guards the tests below against passing on an empty package (a broken import path finds no modules).
    assert MESSAGE_CLASSES[folder]


@pytest.mark.parametrize(("folder", "message_name"), ALL_MESSAGES)
def test_every_message_round_trips_byte_identical(folder: str, message_name: str, fixtures_folder: Path) -> None:
    golden_bytes = read_golden_bytes(fixtures_folder, folder, message_name)

    message = MESSAGE_CLASSES[folder][message_name].FromString(golden_bytes)
    message.DiscardUnknownFields()

    assert message.SerializeToString(deterministic=True) == golden_bytes


@pytest.mark.parametrize("folder", sorted(GOLDEN_PACKAGES))
def test_every_golden_names_a_message(folder: str, fixtures_folder: Path) -> None:
    # Only the folder's own files: subfolders such as without_stability/ hold extra goldens.
    golden_names = {path.stem for path in (fixtures_folder / GOLDEN_ROOT / folder).glob(f"*{JSON_SUFFIX}")}
    assert golden_names == set(MESSAGE_CLASSES[folder])


def test_descriptors_with_stability_set_decode_it(fixtures_folder: Path) -> None:
    command = CommandDescriptor.FromString(read_golden_bytes(fixtures_folder, "commands", "CommandDescriptor"))
    event_type = EventTypeDescriptor.FromString(read_golden_bytes(fixtures_folder, "events", "EventTypeDescriptor"))

    assert command.stability == STABILITY_PREVIEW
    assert event_type.stability == STABILITY_PREVIEW


def test_descriptors_without_stability_decode_as_the_zero_value(fixtures_folder: Path) -> None:
    # An older daemon sends descriptors without the field. They decode as STABILITY_UNSPECIFIED, which every
    # reader treats as preview (layout 3.2), and they still round-trip unchanged.
    command_bytes = read_golden_bytes(fixtures_folder, f"commands/{WITHOUT_STABILITY_FOLDER}", "CommandDescriptor")
    event_bytes = read_golden_bytes(fixtures_folder, f"events/{WITHOUT_STABILITY_FOLDER}", "EventTypeDescriptor")
    command = CommandDescriptor.FromString(command_bytes)
    event_type = EventTypeDescriptor.FromString(event_bytes)

    assert command.stability == STABILITY_UNSPECIFIED
    assert event_type.stability == STABILITY_UNSPECIFIED
    assert command.SerializeToString(deterministic=True) == command_bytes
    assert event_type.SerializeToString(deterministic=True) == event_bytes
