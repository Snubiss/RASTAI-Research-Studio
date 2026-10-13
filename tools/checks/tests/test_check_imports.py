"""check-imports on passing and failing fixture trees: Python layering and wrappers, JavaScript layering and
public API, Rust wrapper files. Also checks that its tables name every package contracts 2.2 and 2.3 list."""

import json
import re
from pathlib import Path

import pytest

# This file sits in tools/checks/tests/, three folders below the repository root.
CONTRACTS = Path(__file__).resolve().parents[3] / "docs/architecture/contracts.md"
# The first backticked name in a table row: `rastai_models` or `@rastai/web`.
FIRST_NAME_IN_ROW = re.compile(r"^\| `([^`]+)`", re.MULTILINE)


def js_package(folder: str, name: str) -> dict[str, str]:
    return {f"{folder}/package.json": json.dumps({"name": name})}


PASSING_PYTHON = {
    "python/rastai_models/rastai_models/compiler.py": "import json\n\nimport torch\nimport rastai_formats\n"
    "from rastai_plugin.registry import PluginRegistry\nfrom rastai_models.blocks import attention\n",
    "python/rastai_models/rastai_models/onnx_export.py": "import onnx\nimport torch.onnx\n",
    "python/rastai_data/rastai_data/image_decode.py": "from torchvision.io import decode_image\n",
    "python/rastai_worker/rastai_worker/distributed.py": "import torch.distributed as dist\n",
    "python/rastai/rastai/connect_transport.py": "from rastai_proto.rastai.events.v1 import events_pb2\n",
    "python/rastai_models/tests/test_compiler.py": "import pytest\n\nfrom rastai_models import compiler\n",
    "python/rastai_proto/tests/test_services.py": "import grpc\n\nfrom rastai_proto.node.v1 import jobs_pb2\n",
    "python/rastai_plugin/rastai_plugin/host.py": "import grpc\nfrom rastai_proto.worker.v1 import worker_pb2\n",
    "python/rastai_plugin/tests/test_host.py": "import grpc\n\nfrom rastai_plugin import WorkerHost\n",
    "python/rastai_worker/tests/test_worker_server.py": "import grpc\n\nfrom rastai_worker import worker_server\n",
    "python/conftest.py": "import pytest\n",
    "python/envs/tests/test_cpu_environment.py": "import torch\nimport grpc\n",
    "plugins/evaluation/python/rastai_eval/rastai_eval/metrics.py": "import rastai\nfrom rastai_plugin import Metric\n",
}
# (path, source, expected text in the output): each must fail on its own.
FAILING_PYTHON = [
    ("python/rastai_lens/rastai_lens/probes.py", "import rastai_train\n", "`rastai_lens` may import only"),
    (
        "python/rastai_rl/rastai_rl/rewards.py",
        "from rastai_eval.metrics import pass_at_k\n",
        "`rastai_rl` may import only",
    ),
    ("python/rastai_train/rastai_train/loop.py", "import rastai_proto\n", "`rastai_train` may import only"),
    ("python/rastai/rastai/load.py", "import rastai_proto\n", "`rastai` may import only"),
    ("python/rastai_data/rastai_data/loader.py", "import torchvision\n", "`torchvision` is imported only in"),
    (
        "python/rastai_train/rastai_train/loop.py",
        "from torch import distributed\n",
        "`torch.distributed` is imported only",
    ),
    ("python/rastai_data/rastai_data/http.py", "import requests\n", "not a §27 library"),
    ("python/rastai_data/rastai_data/arrays.py", "import numpy\n", "arrives only as a transitive package"),
    (
        "python/rastai_models/rastai_models/io/safetensors_reader.py",
        "import struct\n",
        "rastai-formats owns the safetensors",
    ),
    ("python/rastai_models/rastai_models/helpers.py", "import pytest\n", "`pytest` is for tests only"),
    # Only the tests of a package that owns the library throughout (rastai_proto/*) may import it.
    ("python/rastai_data/tests/test_loader.py", "import grpc\n", "`grpc` is imported only in"),
    # rastai_plugin's other files and tests call the host client instead.
    ("python/rastai_plugin/rastai_plugin/registry.py", "import grpc\n", "`grpc` is imported only in"),
    ("python/rastai_plugin/tests/test_registry.py", "import grpc\n", "`grpc` is imported only in"),
    # rastai_worker's other files and tests go through worker_server.py and rastai_plugin's host client.
    ("python/rastai_worker/rastai_worker/launch.py", "import grpc\n", "`grpc` is imported only in"),
    ("python/rastai_worker/tests/test_control.py", "import grpc\n", "`grpc` is imported only in"),
    ("python/rastai_newthing/rastai_newthing/a.py", "import json\n", "package `rastai_newthing` isn't in contracts"),
]

DIAGNOSTIC_VIEW = """/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
import { useState } from "react";
import { findEntry } from "@rastai/catalog";
import { Panel } from "../components/Panel.jsx";
"""
CANVAS_COLORS = """/** @returns {import("@rastai/canvas").CanvasColors} */
export function canvasColors() {}
"""
CONNECTION = """import { createClient } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
"""
EXECUTE = """import { DIAGNOSTIC_CODES } from "./constants.js";
const url = "http://localhost"; // a string that isn't an import
"""
EXECUTE_TEST = """import { expect, test } from "vitest";
import { readFileSync } from "node:fs";
"""
FAKE_DAEMON = """import { create } from "@bufbuild/protobuf";
import { createConnectRouter } from "@connectrpc/connect";
import { createFetchHandler } from "@connectrpc/connect/protocol";
"""
PROTO_GOLDEN_TEST = """import { toBinary } from "@bufbuild/protobuf";
import { expect, test } from "vitest";
"""
BOOT = """import { connectToDaemon } from "@rastai/client";
import { EgressBadge } from "@rastai/ui/security";
const lazy = () => import("./sections/home/Home.jsx");
"""
HOME_SECTION = """import { usePluginHost } from "@rastai/plugin";
import { Button } from "@rastai/ui";
"""
PLUGIN_INDEX = """export * from "@rastai/client/commands";
export { createMockHost } from "./testing/mock-host.js";
"""
PLUGIN_CONTRACT_TEST = """import { connectToDaemon } from "@rastai/client";
import { EXECUTOR_CONTRACT_CASES } from "@rastai/client/testing";
import { createFakeDaemon } from "@rastai/client/testing/fake-daemon";
import { expect, test } from "vitest";
"""
PLUGIN_RENDER = """import { act } from "react";
import { createRoot } from "react-dom/client";
"""
PASSING_JAVASCRIPT = (
    js_package("packages/ui", "@rastai/ui")
    | js_package("packages/client", "@rastai/client")
    | js_package("packages/viz", "@rastai/viz")
    | js_package("apps/web", "@rastai/web")
    | js_package("packages/plugin", "@rastai/plugin")
    | {
        "packages/ui/src/diagnostics/DiagnosticView.jsx": DIAGNOSTIC_VIEW,
        "packages/ui/src/tokens/canvas-colors.js": CANVAS_COLORS,
        "packages/client/src/connection.js": CONNECTION,
        "packages/client/src/execute.js": EXECUTE,
        "packages/client/src/execute.test.js": EXECUTE_TEST,
        "packages/client/src/testing/fake-daemon.js": FAKE_DAEMON,
        "packages/client/src/proto-golden.test.js": PROTO_GOLDEN_TEST,
        "packages/viz/src/flow/data-flow-view.js": 'import { createContext } from "@rastai/canvas";\n',
        "apps/web/src/app/boot.js": BOOT,
        "apps/web/src/sections/home/Home.jsx": HOME_SECTION,
        "packages/plugin/src/index.js": PLUGIN_INDEX,
        "packages/plugin/src/testing/host-contract.test.js": PLUGIN_CONTRACT_TEST,
        "packages/plugin/src/testing/render.js": PLUGIN_RENDER,
        # beside src/: dev tooling, not package code
        "apps/web/vite.config.js": 'import { defineConfig } from "vite";\n',
    }
)
FAILING_JAVASCRIPT = [
    ("apps/web/src/sections/home/Home.jsx", 'import { DaemonClient } from "@rastai/client";\n', "here may import only"),
    ("apps/web/src/state/store.js", 'import { DaemonClient } from "@rastai/client";\n', "here may import only"),
    (
        "plugins/psychology/ui/src/Live.jsx",
        'import { TrustPrompt } from "@rastai/ui/security";\n',
        "only apps/web/src/ may import",
    ),
    # A plugin UI can't render the security UI, so it can't fake it (frontend layout 2.10): not by a static
    # import, a lazy one or a re-export, and @rastai/plugin can't hand it on either.
    (
        "plugins/evaluation/ui/src/SuitePanel.jsx",
        'const prompt = await import("@rastai/ui/security");\n',
        "only apps/web/src/ may import",
    ),
    (
        "plugins/evaluation/ui/src/index.js",
        'export { PermissionList } from "@rastai/ui/security";\n',
        "only apps/web/src/ may import",
    ),
    (
        "packages/plugin/src/index.js",
        'export { EgressBadge, TrustPrompt } from "@rastai/ui/security";\n',
        "only apps/web/src/ may import",
    ),
    # A comment inside the parentheses or between `from` and the string doesn't hide the import: the check
    # reads the code around it, and a comment is only whitespace there (FU-273).
    (
        "plugins/psychology/ui/src/Live.jsx",
        'const prompt = await import(/* @vite-ignore */ "@rastai/ui/security");\n',
        "only apps/web/src/ may import",
    ),
    (
        "plugins/psychology/ui/src/Live.jsx",
        'const prompt = await import(// @vite-ignore\n"@rastai/ui/security");\n',
        "only apps/web/src/ may import",
    ),
    (
        "plugins/evaluation/ui/src/index.js",
        'export { PermissionList } from /* re-export */ "@rastai/ui/security";\n',
        "only apps/web/src/ may import",
    ),
    (
        "packages/plugin/src/loader.js",
        'const prompt = await import(/* @vite-ignore */ "@rastai/ui/security");\n',
        "only apps/web/src/ may import",
    ),
    (
        "packages/plugin/src/loader.js",
        'const prompt = await import(// @vite-ignore\n"@rastai/ui/security");\n',
        "only apps/web/src/ may import",
    ),
    (
        "packages/plugin/src/index.js",
        'export { TrustPrompt } from /* re-export */ "@rastai/ui/security";\n',
        "only apps/web/src/ may import",
    ),
    # Sections reach the typed wrappers through @rastai/plugin, and only the plugin hosts' tests use the client's
    # test support.
    (
        "apps/web/src/sections/home/Home.jsx",
        'import { jobList } from "@rastai/client/commands";\n',
        "only packages/plugin/src/ may import",
    ),
    (
        "packages/plugin/src/react/use-query.js",
        'import { createFakeDaemon } from "@rastai/client/testing/fake-daemon";\n',
        "only packages/plugin/src/testing/ may import",
    ),
    (
        "packages/plugin/src/react/CommandForm.jsx",
        'import { createRoot } from "react-dom/client";\n',
        "`react-dom` is imported only in",
    ),
    (
        "packages/ui/src/diagnostics/View.jsx",
        'import { Diagnostic } from "@rastai/proto";\n',
        "only in a JSDoc type import",
    ),
    ("packages/ui/src/hooks/use-daemon.js", 'import { DaemonClient } from "@rastai/client";\n', "here may import only"),
    (
        "apps/web/src/sections/home/Home.jsx",
        'import { host } from "@rastai/plugin/src/host.js";\n',
        "its files aren't public API",
    ),
    (
        "packages/viz/src/chart/axes.js",
        'import { GraphRenderer } from "../../../canvas/src/graph-renderer.js";\n',
        "reaches outside its package",
    ),
    ("packages/canvas/src/graph-renderer.js", 'import { useState } from "react";\n', "`react` is imported only in"),
    (
        "packages/client/src/execute.js",
        'import { Code } from "@connectrpc/connect";\n',
        "`@connectrpc/connect` is imported only in",
    ),
    # The client's other tests go through the fake daemon; only proto-golden.test.js re-encodes with protobuf-es.
    (
        "packages/client/src/execute.test.js",
        'import { createRouterTransport } from "@connectrpc/connect";\n',
        "`@connectrpc/connect` is imported only in",
    ),
    (
        "packages/client/src/event-stream.test.js",
        'import { create } from "@bufbuild/protobuf";\n',
        "`@bufbuild/protobuf` is imported only in",
    ),
    (
        "packages/client/src/testing/executor-contract.js",
        'import { createConnectTransport } from "@connectrpc/connect-web";\n',
        "`@connectrpc/connect-web` is imported only in",
    ),
    ("packages/client/src/login.js", 'import { readFileSync } from "node:fs";\n', "is for tests only"),
    ("packages/ui/src/components/Table.jsx", 'import sortBy from "lodash/sortBy";\n', "not a §27 library"),
    (
        "packages/canvas/src/index.js",
        'export { LineChart } from "@rastai/viz";\n',
        "`@rastai/canvas` here may import only no other package",
    ),
]


def test_passing_python_tree(write_tree, run_check):
    root = write_tree(PASSING_PYTHON)
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 0, result.stdout


@pytest.mark.parametrize(("path", "source", "expected"), FAILING_PYTHON)
def test_each_python_rule_fails(write_tree, run_check, path, source, expected):
    root = write_tree({path: source})
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    assert f"{path}:" in result.stdout and expected in result.stdout, result.stdout


def test_passing_javascript_tree(write_tree, run_check):
    root = write_tree(PASSING_JAVASCRIPT)
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 0, result.stdout


@pytest.mark.parametrize(("path", "source", "expected"), FAILING_JAVASCRIPT)
def test_each_javascript_rule_fails(write_tree, run_check, path, source, expected):
    packages = (
        js_package("packages/ui", "@rastai/ui")
        | js_package("packages/client", "@rastai/client")
        | js_package("packages/viz", "@rastai/viz")
        | js_package("packages/canvas", "@rastai/canvas")
        | js_package("apps/web", "@rastai/web")
        | js_package("plugins/psychology/ui", "@rastai/psychology-ui")
        | js_package("plugins/evaluation/ui", "@rastai/eval-ui")
        | js_package("packages/plugin", "@rastai/plugin")
    )
    root = write_tree(packages | {path: source})
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    # The specifier is the first double-quoted string in each case, so its line is the lines before it, plus one.
    line = 1 + source.split('"')[0].count("\n")
    assert f"{path}:{line}: imports" in result.stdout and expected in result.stdout, result.stdout


def test_an_unknown_javascript_package_fails(write_tree, run_check):
    root = write_tree(js_package("packages/charts", "@rastai/charts"))
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    assert "package `@rastai/charts` isn't in contracts 2.3" in result.stdout


MODEL_HASH_USING_BLAKE3 = """fn hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}
"""


def test_rust_wrapper_files(write_tree, run_check):
    root = write_tree(
        {
            "crates/rastai-formats/src/hashing.rs": "use blake3::Hasher;\n",
            "crates/rastai-formats/src/json/canonical.rs": "// hashed through crate::hashing, never blake3::hash\n"
            "use crate::hashing::hash_bytes;\n",
            "crates/rastai-formats/src/model_hash.rs": MODEL_HASH_USING_BLAKE3,
            # A path inside a string or a block comment isn't a use.
            "crates/rastai-formats/src/messages.rs": 'const HINT: &str = "call blake3::hash";\n/* rusqlite::open */\n',
        }
    )
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    assert "crates/rastai-formats/src/model_hash.rs:2: uses `blake3`, which only" in result.stdout
    assert "1 problem(s)" in result.stdout


def test_http_body_is_used_only_by_the_connect_file(write_tree, run_check):
    """The frame type of Subscribe's streamed body comes from http-body, in rastai-server's http/connect.rs alone."""
    root = write_tree(
        {
            "crates/rastai-server/src/http/connect.rs": "use http_body::Frame;\n",
            "crates/rastai-server/src/http/event_endpoint.rs": "use http_body::Frame;\n",
        }
    )
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    assert "crates/rastai-server/src/http/event_endpoint.rs:1: uses `http_body`, which only" in result.stdout
    assert "1 problem(s)" in result.stdout


def test_a_rust_use_after_a_double_slash_in_a_string_fails(write_tree, run_check):
    """`//` inside a string starts no comment, so the blake3 call after it on the same line is still seen."""
    root = write_tree({"crates/rastai-formats/src/digest.rs": 'let d = "x//y"; blake3::hash(b"a");\n'})
    result = run_check("check-imports", "--root", str(root))
    assert result.returncode == 1
    assert "crates/rastai-formats/src/digest.rs:1: uses `blake3`" in result.stdout


def test_every_contracts_package_has_a_row(load_check):
    """Contracts 2.2 and 2.3 are the source; the check's tables must name every package they list."""
    check = load_check("check-imports")
    text = CONTRACTS.read_text()
    python_table = text[text.index("### 2.2 Python packages") : text.index("### 2.3 Apps and JavaScript packages")]
    javascript_table = text[text.index("### 2.3 Apps and JavaScript packages") : text.index("### 2.4 Layering rules")]
    python_packages = set(FIRST_NAME_IN_ROW.findall(python_table))
    javascript_packages = set(FIRST_NAME_IN_ROW.findall(javascript_table))
    assert python_packages and javascript_packages
    assert python_packages == set(check.PYTHON_INTERNAL_DEPS)
    assert javascript_packages == set(check.JS_INTERNAL_DEPS)


def test_the_real_repository_passes(run_check):
    result = run_check("check-imports")
    assert result.returncode == 0, result.stdout
