"""Golden messages (layout: rust-node-gateway-proto.md section 3.5).

Every message of rastai.common.v1 has a golden in fixtures/proto-golden/common/, and its .binpb decodes and encodes
back to exactly the same bytes. The Rust and JavaScript tests compare against the same files, so the three encoders
agree byte for byte. Python keeps fields it doesn't know and writes them back, which would hide a module that lags
behind the .proto, so the test drops unknown fields before encoding.
"""

import importlib
import pkgutil
from pathlib import Path
from types import ModuleType

import pytest
import rastai_proto.common.v1

GOLDEN_FOLDER = Path("proto-golden") / "common"
BINARY_SUFFIX = ".binpb"
JSON_SUFFIX = ".json"
# Generated message modules end in _pb2; the _pb2_grpc modules beside them hold service stubs only.
MESSAGE_MODULE_SUFFIX = "_pb2"


def common_message_modules() -> list[ModuleType]:
    """Every generated message module of rastai_proto.common.v1, found on disk so a new .proto is never missed."""
    package = rastai_proto.common.v1
    modules = []
    for module_info in pkgutil.iter_modules(package.__path__):
        if module_info.name.endswith(MESSAGE_MODULE_SUFFIX):
            modules.append(importlib.import_module(f"{package.__name__}.{module_info.name}"))
    return modules


def common_message_classes() -> dict[str, type]:
    """Message name -> generated class, for every top-level message of the package."""
    classes = {}
    for module in common_message_modules():
        for name in module.DESCRIPTOR.message_types_by_name:
            classes[name] = getattr(module, name)
    return classes


COMMON_MESSAGE_CLASSES = common_message_classes()


def test_the_package_has_messages() -> None:
    # Guards the two tests below against passing on an empty package (a broken import path finds no modules).
    assert "Diagnostic" in COMMON_MESSAGE_CLASSES


@pytest.mark.parametrize("message_name", sorted(COMMON_MESSAGE_CLASSES))
def test_every_message_round_trips_byte_identical(message_name: str, fixtures_folder: Path) -> None:
    golden_path = fixtures_folder / GOLDEN_FOLDER / f"{message_name}{BINARY_SUFFIX}"
    assert golden_path.is_file(), f"{message_name} has no golden; add {golden_path.name} (fixtures/proto-golden/)"
    golden_bytes = golden_path.read_bytes()

    message = COMMON_MESSAGE_CLASSES[message_name].FromString(golden_bytes)
    message.DiscardUnknownFields()

    assert message.SerializeToString(deterministic=True) == golden_bytes


def test_every_golden_names_a_message(fixtures_folder: Path) -> None:
    golden_names = {path.stem for path in (fixtures_folder / GOLDEN_FOLDER).glob(f"*{JSON_SUFFIX}")}
    assert golden_names == set(COMMON_MESSAGE_CLASSES)
