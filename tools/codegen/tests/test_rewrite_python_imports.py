"""Tests for tools/codegen/rewrite_python_imports, buf's Python plugin (layout rust-node-gateway-proto.md 3.5).

The line rules are tested on their own; then the whole plugin runs on a request with two packages, one importing
the other and holding a service, and the generated package must import cleanly as rastai_proto.
"""

import subprocess
import sys
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
from types import ModuleType

import pytest
from google.protobuf import descriptor_pb2
from google.protobuf.compiler import plugin_pb2

# This file sits in tools/codegen/tests/.
PLUGIN_PATH = Path(__file__).resolve().parent.parent / "rewrite_python_imports"
FIELD = descriptor_pb2.FieldDescriptorProto
# The two files of the request: beta.v1 imports alpha.v1, so the generated beta module imports the alpha one.
INNER_FILE_NAME = "rastai/alpha/v1/inner.proto"
OUTER_FILE_NAME = "rastai/beta/v1/outer.proto"
# Run in a fresh interpreter whose import path starts with the generated tree, so this package (not the
# checked-in rastai_proto) is the one imported.
IMPORT_CHECK = """
from rastai_proto.alpha.v1 import inner_pb2
from rastai_proto.beta.v1 import outer_pb2, outer_pb2_grpc

outer = outer_pb2.Outer(inner=inner_pb2.Inner(label="ok"))
decoded = outer_pb2.Outer.FromString(outer.SerializeToString())
assert decoded.inner.label == "ok"
assert type(decoded).__module__ == "rastai_proto.beta.v1.outer_pb2"
assert outer_pb2.DESCRIPTOR.package == "rastai.beta.v1"
assert hasattr(outer_pb2_grpc, "EchoServiceServicer")
print("imported")
"""


def load_plugin() -> ModuleType:
    """The plugin as a module, for tests of its functions. It has no .py ending (buf runs it by path)."""
    loader = SourceFileLoader("rewrite_python_imports", str(PLUGIN_PATH))
    module = module_from_spec(spec_from_loader("rewrite_python_imports", loader))
    loader.exec_module(module)
    return module


PLUGIN = load_plugin()


# ---- The line rules ----


def test_an_import_of_another_package_is_rewritten() -> None:
    line = "from rastai.common.v1 import diagnostic_pb2 as rastai_dot_common_dot_v1_dot_diagnostic__pb2\n"
    expected = "from rastai_proto.common.v1 import diagnostic_pb2 as rastai_dot_common_dot_v1_dot_diagnostic__pb2\n"
    assert PLUGIN.rewrite_line(line) == expected


def test_the_registered_module_name_is_rewritten() -> None:
    line = "_builder.BuildTopDescriptorsAndMessages(DESCRIPTOR, 'rastai.common.v1.blob_pb2', _globals)\n"
    expected = "_builder.BuildTopDescriptorsAndMessages(DESCRIPTOR, 'rastai_proto.common.v1.blob_pb2', _globals)\n"
    assert PLUGIN.rewrite_line(line) == expected


@pytest.mark.parametrize(
    "line",
    [
        "from google.protobuf import descriptor as _descriptor\n",
        "DESCRIPTOR = _descriptor_pool.Default().AddSerializedFile(b'\\n\\x1brastai.common.v1')\n",
        "        + ' but the generated code in rastai/common/v1/blob_pb2_grpc.py depends on'\n",
        "    rastai_dot_common_dot_v1_dot_blob__pb2.BlobRef.SerializeToString,\n",
    ],
)
def test_other_lines_are_left_alone(line: str) -> None:
    # The serialized descriptor keeps the proto package name: it is the wire name, not a Python module.
    assert PLUGIN.rewrite_line(line) == line


def test_files_move_out_of_the_rastai_folder() -> None:
    assert PLUGIN.package_path("rastai/common/v1/diagnostic_pb2.py") == "common/v1/diagnostic_pb2.py"


def test_a_file_outside_rastai_is_refused() -> None:
    with pytest.raises(ValueError, match="not under rastai/"):
        PLUGIN.package_path("google/protobuf/empty_pb2.py")


def test_every_folder_gets_an_init_file() -> None:
    folders = PLUGIN.package_folders({"common/v1/a_pb2.py": "", "node/v1/b_pb2.py": ""})
    assert folders == ["", "common/", "common/v1/", "node/", "node/v1/"]


# ---- The whole plugin ----


def build_request() -> plugin_pb2.CodeGeneratorRequest:
    inner = descriptor_pb2.FileDescriptorProto(name=INNER_FILE_NAME, package="rastai.alpha.v1", syntax="proto3")
    inner_message = inner.message_type.add(name="Inner")
    inner_message.field.add(
        name="label", json_name="label", number=1, type=FIELD.TYPE_STRING, label=FIELD.LABEL_OPTIONAL
    )

    outer = descriptor_pb2.FileDescriptorProto(
        name=OUTER_FILE_NAME, package="rastai.beta.v1", syntax="proto3", dependency=[INNER_FILE_NAME]
    )
    outer_message = outer.message_type.add(name="Outer")
    outer_message.field.add(
        name="inner",
        json_name="inner",
        number=1,
        type=FIELD.TYPE_MESSAGE,
        type_name=".rastai.alpha.v1.Inner",
        label=FIELD.LABEL_OPTIONAL,
    )
    service = outer.service.add(name="EchoService")
    service.method.add(name="Echo", input_type=".rastai.beta.v1.Outer", output_type=".rastai.beta.v1.Outer")
    return plugin_pb2.CodeGeneratorRequest(
        file_to_generate=[INNER_FILE_NAME, OUTER_FILE_NAME], proto_file=[inner, outer]
    )


def run_plugin(request: plugin_pb2.CodeGeneratorRequest) -> plugin_pb2.CodeGeneratorResponse:
    """Runs the plugin the way buf does: the request on stdin, the response on stdout."""
    result = subprocess.run(
        [sys.executable, str(PLUGIN_PATH)], input=request.SerializeToString(), capture_output=True, check=True
    )
    return plugin_pb2.CodeGeneratorResponse.FromString(result.stdout)


def write_response(response: plugin_pb2.CodeGeneratorResponse, package_folder: Path) -> None:
    for generated_file in response.file:
        path = package_folder / generated_file.name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(generated_file.content)


def test_the_generated_package_imports_cleanly(tmp_path: Path) -> None:
    response = run_plugin(build_request())
    assert response.error == ""
    assert response.supported_features & plugin_pb2.CodeGeneratorResponse.FEATURE_PROTO3_OPTIONAL
    write_response(response, tmp_path / "rastai_proto")

    result = subprocess.run(
        [sys.executable, "-c", IMPORT_CHECK], cwd=tmp_path, env={"PYTHONPATH": str(tmp_path)}, capture_output=True
    )

    assert result.returncode == 0, result.stderr.decode()
    assert result.stdout.decode().strip() == "imported"


def test_no_generated_line_imports_rastai() -> None:
    response = run_plugin(build_request())
    names = sorted(generated_file.name for generated_file in response.file)
    assert names == [
        "__init__.py",
        "alpha/__init__.py",
        "alpha/v1/__init__.py",
        "alpha/v1/inner_pb2.py",
        "alpha/v1/inner_pb2_grpc.py",
        "beta/__init__.py",
        "beta/v1/__init__.py",
        "beta/v1/outer_pb2.py",
        "beta/v1/outer_pb2_grpc.py",
    ]
    for generated_file in response.file:
        for line in generated_file.content.splitlines():
            assert not line.startswith(("from rastai.", "import rastai")), f"{generated_file.name}: {line}"
