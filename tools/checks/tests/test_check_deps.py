"""check-deps on passing and failing fixture trees: Cargo, uv and pnpm, each against its allowlist."""

import json

import pytest

ALLOWED_LICENSES = ["Apache-2.0", "MIT", "BSD-3-Clause"]
CARGO_ALLOWLIST = {
    "serde": {"category": "foundation", "reason": "documents", "users": ["*"]},
    "blake3": {"category": "security", "reason": "hashes", "users": ["rastai-formats"]},
    "libfuzzer-sys": {"category": "dev", "reason": "fuzz targets", "users": ["*"]},
}
PYTHON_ALLOWLIST = {
    "locks": ["python/envs/cpu/uv.lock"],
    "licenses": {
        "allowed": ALLOWED_LICENSES,
        "named_entries": {"torch": {"license": "Apache-2.0 WITH LLVM-exception", "reason": "vendored LLVM code"}},
    },
    "packages": {
        "torch": {
            "version": "2.14.1",
            "category": "foundation",
            "license": "BSD-3-Clause AND Apache-2.0 WITH LLVM-exception",
        },
        "filelock": {"version": "4.0.4", "category": "transitive", "license": "MIT", "brought_by": ["torch"]},
    },
}
UV_LOCK = """version = 1

[[package]]
name = "filelock"
version = "4.0.4"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "rastai-env-cpu"
version = "0.1.0"
source = { virtual = "." }

[[package]]
name = "torch"
version = "2.14.1+cpu"
source = { registry = "https://download.pytorch.org/whl/cpu" }
"""
NPM_ALLOWLIST = {
    "locks": ["pnpm-lock.yaml"],
    "licenses": {"allowed": ALLOWED_LICENSES, "named_entries": {}},
    "packages": {"vitest": {"version": "5.0.2", "category": "dev", "license": "MIT", "reason": "tests"}},
}
PNPM_LOCK = """lockfileVersion: '9.0'

importers:

  .:
    devDependencies:
      vitest:
        specifier: 5.0.2
        version: 5.0.2(vite@8.3.1)

  packages/ui:
    dependencies:
      '@rastai/catalog':
        specifier: workspace:*
        version: link:../catalog

packages:

  '@vitest/spy@5.0.2':
    resolution: {integrity: sha512-x}

  vitest@5.0.2:
    resolution: {integrity: sha512-y}
"""


def installed_manifest(name: str, version: str, license_name: str) -> str:
    return json.dumps({"name": name, "version": version, "license": license_name})


def cargo_metadata(dependencies: list[dict]) -> str:
    member = {"id": "member-id", "name": "rastai-formats", "dependencies": dependencies}
    return json.dumps(
        {
            "packages": [member],
            "workspace_members": ["member-id"],
            "metadata": {"rastai": {"allowlist": CARGO_ALLOWLIST}},
        }
    )


def crate(name: str, kind: str | None = None) -> dict:
    return {"name": name, "source": "registry+https://github.com/rust-lang/crates.io-index", "kind": kind}


def passing_files() -> dict[str, str]:
    return {
        "cargo-metadata.json": cargo_metadata([crate("serde"), crate("blake3"), crate("libfuzzer-sys", "dev")]),
        "deps/allowlist-python.json": json.dumps(PYTHON_ALLOWLIST),
        "deps/allowlist-npm.json": json.dumps(NPM_ALLOWLIST),
        "python/envs/cpu/uv.lock": UV_LOCK,
        "pnpm-lock.yaml": PNPM_LOCK,
        "node_modules/.pnpm/vitest@5.0.2_vite@8.3.1/node_modules/vitest/package.json": installed_manifest(
            "vitest", "5.0.2", "MIT"
        ),
        "node_modules/.pnpm/@vitest+spy@5.0.2/node_modules/@vitest/spy/package.json": installed_manifest(
            "@vitest/spy", "5.0.2", "MIT"
        ),
    }


def run_deps(run_check, root):
    return run_check("check-deps", "--root", str(root), "--cargo-metadata", str(root / "cargo-metadata.json"))


def test_passing_tree(write_tree, run_check):
    result = run_deps(run_check, write_tree(passing_files()))
    assert result.returncode == 0, result.stdout
    assert "npm licenses read for 2 of 2 locked packages" in result.stdout


@pytest.mark.parametrize(
    ("dependencies", "expected"),
    [
        (
            [crate("reqwest")],
            "rastai-formats: depends on `reqwest`, which isn't in [workspace.metadata.rastai.allowlist]",
        ),
        (
            [crate("libfuzzer-sys")],
            'rastai-formats: `libfuzzer-sys` is dev-only (category "dev") but sits in [dependencies]',
        ),
    ],
)
def test_cargo_problems_fail(write_tree, run_check, dependencies, expected):
    files = passing_files() | {"cargo-metadata.json": cargo_metadata(dependencies)}
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert expected in result.stdout


def test_a_crate_used_by_a_member_it_doesnt_list_fails(write_tree, run_check):
    metadata = json.loads(cargo_metadata([crate("blake3")]))
    metadata["packages"][0]["name"] = "rastai-core"
    files = passing_files() | {"cargo-metadata.json": json.dumps(metadata)}
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert "rastai-core: depends on `blake3`, but the allowlist names only rastai-formats as its users" in result.stdout


@pytest.mark.parametrize(
    ("lock_change", "expected"),
    [
        (('version = "4.0.4"', 'version = "4.1.0"'), "`filelock` is 4.1.0, but deps/allowlist-python.json lists 4.0.4"),
        (('name = "filelock"', 'name = "requests"'), "`requests` 4.0.4 isn't in deps/allowlist-python.json"),
    ],
)
def test_python_lock_problems_fail(write_tree, run_check, lock_change, expected):
    old_text, new_text = lock_change
    files = passing_files() | {"python/envs/cpu/uv.lock": UV_LOCK.replace(old_text, new_text)}
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert expected in result.stdout


def test_a_python_license_outside_the_list_fails(write_tree, run_check):
    allowlist = json.loads(json.dumps(PYTHON_ALLOWLIST))
    allowlist["packages"]["filelock"]["license"] = "GPL-3.0-only"
    files = passing_files() | {"deps/allowlist-python.json": json.dumps(allowlist)}
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert "`filelock` has license GPL-3.0-only, which `licenses` doesn't allow" in result.stdout


def test_npm_problems_fail(write_tree, run_check):
    files = passing_files() | {
        "pnpm-lock.yaml": PNPM_LOCK.replace("version: 5.0.2(vite@8.3.1)", "version: 5.1.0").replace(
            "      vitest:\n", "      left-pad:\n        specifier: 1.3.0\n        version: 1.3.0\n      vitest:\n"
        ),
        "node_modules/.pnpm/@vitest+spy@5.0.2/node_modules/@vitest/spy/package.json": installed_manifest(
            "@vitest/spy", "5.0.2", "GPL-3.0-only"
        ),
    }
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert "pnpm-lock.yaml: `vitest` is 5.1.0, but deps/allowlist-npm.json lists 5.0.2" in result.stdout
    assert "pnpm-lock.yaml: `left-pad` 1.3.0 isn't in deps/allowlist-npm.json" in result.stdout
    assert "npm package `@vitest/spy` 5.0.2 has license GPL-3.0-only" in result.stdout


def test_a_listed_package_no_lock_holds_fails(write_tree, run_check):
    allowlist = json.loads(json.dumps(NPM_ALLOWLIST))
    allowlist["packages"]["prettier"] = {"version": "3.9.9", "category": "dev", "license": "MIT", "reason": "format"}
    files = passing_files() | {"deps/allowlist-npm.json": json.dumps(allowlist)}
    result = run_deps(run_check, write_tree(files))
    assert result.returncode == 1
    assert "`prettier` is listed, but no package.json depends on it" in result.stdout


@pytest.mark.parametrize(
    ("expression", "package_name", "passes"),
    [
        ("MIT", "any", True),
        ("(MIT OR GPL-3.0-only)", "any", True),
        ("MIT AND GPL-3.0-only", "any", False),
        ("Apache-2.0 WITH LLVM-exception", "torch", True),
        ("Apache-2.0 WITH LLVM-exception", "other", False),
        ("BSD-3-Clause AND (Apache-2.0 OR MIT)", "any", True),
        ("(MIT", "any", False),
        ("", "any", False),
    ],
)
def test_license_expressions(load_check, expression, package_name, passes):
    check = load_check("check-deps")
    assert check.license_passes(expression, package_name, PYTHON_ALLOWLIST["licenses"]) == passes
