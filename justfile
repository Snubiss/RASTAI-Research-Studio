# Every build, test and check task goes through `just` (layout: rust-control-plane.md section 2.5).
# "Done" means `just check` passes (§29). Running `just` with no recipe runs `check`.
# Run `just install-hooks` once per clone: git then runs `just check` before every push and refuses a push that
# fails it (tools/hooks/pre-push; plan 3.6).
#
# Dev tools, pinned. Install them for your user (no sudo) with:
#   cargo install --locked just@1.58.0
#   cargo install --locked cargo-deny@0.20.2
#   cargo install --locked cargo-fuzz@0.13.2       (only for `just fuzz`)
#   rustup toolchain install nightly-2026-09-27 --profile minimal   (only for `just fuzz`)
# cargo install checks each crate against the checksum in the crates.io index.
# The Rust toolchain is pinned in rust-toolchain.toml; rustup installs it on first use. Building the workspace also needs
# a python3 on PATH (any 3.x): pyo3's build script asks it how to build rastai-formats-python (.cargo/config.toml). Fuzzing needs nightly
# (cargo-fuzz builds with sanitizer flags), so its toolchain is pinned here instead: fuzz_toolchain below.
#
# Python and JavaScript tools, also for your user only:
#   uv 0.13.0, which `required-version` in python/pyproject.toml and python/envs/cpu/pyproject.toml enforces.
#     Download https://github.com/astral-sh/uv/releases/download/0.13.0/uv-x86_64-unknown-linux-gnu.tar.gz,
#     check it with `sha256sum`: 1468ebd5a5541121837c5a2817b9972ba6090fa6caa3d142620850a47fb75154,
#     and put uv and uvx in ~/.local/bin. `just env-cpu` then fetches Python 3.12 itself; uv checks that download.
#   Node.js 24.21.0, which `engines.node` in package.json pins and `engineStrict` in pnpm-workspace.yaml enforces.
#     Download https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.xz,
#     check it with `sha256sum`: fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6,
#     unpack it into ~/.local/opt and link its bin/node, bin/npm and bin/corepack into ~/.local/bin.
#   Then pnpm through Node's corepack:
#     corepack enable --install-directory ~/.local/bin pnpm
#     package.json `packageManager` pins pnpm 12.7.0 with its sha512, and corepack refuses a download that differs.
#
# Protobuf tools for `just codegen`, `just codegen-check` and `just proto-check`, also for your user only:
#   buf 1.73.0. Download https://github.com/bufbuild/buf/releases/download/v1.73.0/buf-Linux-x86_64,
#     check it with `sha256sum`: 8f2986298ad08f0cc1bf999b9797b7c383adf32d7edf0f73d6f1e1a701baeac1,
#     and install it as ~/.local/bin/buf (chmod +x).
#   The Rust plugins buf runs:
#     cargo install --locked protoc-gen-prost@0.5.0
#     cargo install --locked protoc-gen-tonic@0.5.0
#   The Python and JavaScript plugins need no install of their own: grpcio-tools is in the cpu environment's dev group
#   (python/envs/cpu/uv.lock) and protoc-gen-es in package.json (pnpm-lock.yaml).
#
# CI installs the same versions with tools/ci/install-dev-tools (prebuilt downloads, each checked against its SHA-256);
# it also works here, for example `tools/ci/install-dev-tools just uv node`.

# `deny` refuses any other cargo-deny version, so every machine reads deny.toml the same way.
cargo_deny_version := "0.20.2"
# `fuzz` runs on this exact nightly and cargo-fuzz, so a crash found one night reproduces the next.
fuzz_toolchain := "nightly-2026-09-27"
cargo_fuzz_version := "0.13.2"
# buf and the Rust protoc plugins its codegen runs. `codegen` and `proto-check` refuse other versions, so every
# machine generates the same bytes and lints by the same rules.
buf_version := "1.73.0"
protoc_gen_prost_version := "0.5.0"
protoc_gen_tonic_version := "0.5.0"
# The branch this one merges into: RASTAI_BASE_REF when set (CI sets it to the pull request's base), else origin/main.
# `proto-check` runs `buf breaking` against its proto files at the merge base, `codegen-check` refuses a catalog code
# that was there and is now gone, and `check-schema-diff` diffs the command and event schemas against it.
base_branch := env("RASTAI_BASE_REF", "origin/main")
# The cpu environment for development, CI and the M1 embedded node (contracts 15.8; git-ignored).
cpu_environment := justfile_directory() / ".dev/envs/cpu"
# The cpu environment's interpreter. A virtual environment keeps it in Scripts\ on Windows and bin/ elsewhere; the
# Windows CI job needs it for test-python-portable.
cpu_python := if os() == "windows" { cpu_environment / "Scripts/python.exe" } else { cpu_environment / "bin/python" }
# The repo checks in tools/checks are standard-library Python scripts. They run on the cpu environment's Python,
# so every machine runs them with the same interpreter.
check_python := cpu_python
# The test folders under python/ that test-python-portable runs on macOS and Windows: the cpu environment's smoke
# test and the pure-compute packages' tests (plan 8.7): rastai_formats, rastai_models, rastai_data and rastai_train
# (tests/workspace/ci-workflows.test.js fails if one that exists is missing).
portable_test_folders := "envs/tests rastai_formats/tests rastai_models/tests rastai_data/tests rastai_train/tests"

set shell := ["bash", "-euo", "pipefail", "-c"]

# The repo checks are the ones of layout rust-control-plane.md 2.3. check-dco isn't here: it reads a pull request's
# commit range, which CI's `dco` job passes when CI runs; locally, `just check-dco` checks the commits not yet on main.
# The gate every PR passes: format, lint, the JSDoc type check, tests, the browser build, dependencies, repo checks.
check: fmt-check lint check-js test-rust test-node-jobs test-daemon-jobs test-python test-js wasm deny proto-check \
    check-deps check-unsafe check-no-pickle check-imports check-colors check-no-typescript check-fuzz-locks \
    codegen-check check-names check-schema-diff

# core.hooksPath goes in this clone's own git config (never --global), so other repositories keep their hooks.
# A relative path is read from the top of each working tree, so every worktree of this clone runs its own copy.
# Point git at tools/hooks, so the pre-push hook runs `just check` before every push.
install-hooks:
    git config core.hooksPath tools/hooks

# Build every crate in the workspace. --locked: Cargo.lock is checked in and never silently changes.
build:
    cargo build --workspace --locked

# Unit tests and crates/*/tests/ for the whole workspace.
test-rust:
    cargo test --workspace --locked

# These tests run tests/support/fake_worker.py, a WorkerService in the cpu environment's Python (grpcio and
# rastai_proto), and the real worker's TextCNN train job (tests/train_job.rs), which a plain `cargo test` on a CI
# runner doesn't have, so they are #[ignore]d there (layout rust-node-gateway-proto.md 7.12). They need Linux: the
# node signals process groups and reads /proc.
# rastai-node's job, reattach and train-job tests that go through Register, Hello and Control, with the cpu environment.
test-node-jobs: env-cpu
    cargo test --locked -p rastai-node --test jobs --test reattach --test train_job -- --ignored

# The real TextCNN train job through the daemon: train.start, the embedded node, the run tail, job.cancel and a
# restart mid-run (plan M1-47). Like test-node-jobs, it needs the cpu environment's Python.
# The daemon's #[ignore]d train job tests (crates/rastai-server/tests/train_jobs.rs).
test-daemon-jobs: env-cpu
    cargo test --locked -p rastai-server --test train_jobs -- --ignored

# Rust by rustfmt (rustfmt.toml), Python by ruff (python/pyproject.toml; tools/checks/pyproject.toml,
# tools/codegen/pyproject.toml, tools/fixtures/pyproject.toml, tools/github/pyproject.toml and
# crates/rastai-node/tests/support/ruff.toml extend it), JavaScript, JSON, CSS and YAML by prettier
# (.prettierrc.json, .prettierignore). All three use 120 columns. Generated code is left as its generator wrote it:
# rustfmt doesn't follow include!, ruff skips the generated Python (python/pyproject.toml) and prettier src/generated/.
# Fails if any file isn't formatted.
fmt-check: env-cpu env-js
    cargo fmt --all --check
    "{{cpu_environment}}/bin/ruff" format --check python tools/checks tools/codegen tools/fixtures tools/github \
        crates/rastai-node/tests/support
    pnpm exec prettier --check .

# The rastai-formats-wasm bindings and their wasm-bindgen step join this recipe in M4.
# The crates that also run in the browser must build for wasm32-unknown-unknown (layout sections 3.4 and 4.10).
wasm:
    cargo build -p rastai-diag -p rastai-formats --target wasm32-unknown-unknown --locked

# Rust: the workspace lints (Cargo.toml) and thresholds (clippy.toml). Python: the rules in python/pyproject.toml.
# clippy and ruff check; every warning fails.
lint: env-cpu
    cargo clippy --workspace --all-targets --locked -- -D warnings
    "{{cpu_environment}}/bin/ruff" check python tools/checks tools/codegen tools/fixtures tools/github \
        crates/rastai-node/tests/support

# This is the same `uv sync --frozen` the node runs from M2-16 (the node also leaves out the dev group, so pytest
# and ruff never reach a user's machine). --frozen installs exactly the lock and never resolves, so after one
# online run it works offline from the uv cache.
# --reinstall-package rebuilds rastai-formats' PyO3 module on every sync, so a Rust change is always in the module
# Python imports. uv would otherwise reuse its cached build, which knows nothing of the Rust sources and can't put
# back a module `git clean` removed. With nothing changed the rebuild is a cargo no-op (about 0.3 s).
# Sync the cpu environment from python/envs/cpu/uv.lock into .dev/envs/cpu.
env-cpu:
    UV_PROJECT_ENVIRONMENT="{{cpu_environment}}" uv sync --frozen --reinstall-package rastai-formats \
        --project python/envs/cpu

# --frozen-lockfile installs exactly the lock and fails instead of changing it.
# Install the JavaScript dev tools (typescript checker, vitest, prettier) from pnpm-lock.yaml into node_modules/.
env-js:
    pnpm install --frozen-lockfile

# Settings and markers are in python/pyproject.toml; python/conftest.py skips gpu tests without a device and
# posix_only tests on Windows. tools/checks/tests runs every repo check on passing and failing fixture trees,
# tools/codegen/tests runs buf's Python plugin on a small request, tools/fixtures/tests checks that the fixture
# generators write the committed bytes, and tools/github/tests runs the backlog sync against a fake gh; each has its
# own settings in its pyproject.toml.
# pytest over python/, tools/checks/tests, tools/codegen/tests, tools/fixtures/tests and tools/github/tests in the cpu
# environment.
test-python: env-cpu
    cd python && "{{cpu_python}}" -m pytest
    cd tools/checks && "{{cpu_python}}" -m pytest
    cd tools/codegen && "{{cpu_python}}" -m pytest
    cd tools/fixtures && "{{cpu_python}}" -m pytest
    cd tools/github && "{{cpu_python}}" -m pytest

# The macOS and Windows CI jobs run this (tier 2 keeps compiling and passing, spec §28 risk 13). Tests that need Unix
# sockets, flock or setsid carry the posix_only marker and are deselected here; Linux runs them in test-python.
# pytest over portable_test_folders (the smoke test and the pure-compute packages), without the posix_only tests.
test-python-portable: env-cpu
    cd python && "{{cpu_python}}" -m pytest -m "not posix_only" {{portable_test_folders}}

# The test files are listed in vitest.config.js.
# vitest over the whole JavaScript workspace.
test-js: env-js
    pnpm exec vitest run

# tsc is a checker only (ADR-0036): --noEmit here, and noEmit in jsconfig.base.json too. jsconfig.json covers the
# whole workspace; each package's own jsconfig.json extends the same base, so an editor and this check agree.
# Type-check the JSDoc in every .js and .jsx file.
check-js: env-js
    pnpm exec tsc --noEmit -p jsconfig.json

# maturin is the cpu environment's own (its dev group pins it in uv.lock), the same build backend that builds the
# editable install. --interpreter makes it a CPython 3.12 wheel for this platform (layout 4.7: abi3 off), and
# --locked builds against the checked-in Cargo.lock. The wheel lands in python/rastai_formats/dist/ (git-ignored).
# Build the rastai-formats wheel (python/rastai_formats with its PyO3 module) in release mode.
wheel: env-cpu
    cd python/rastai_formats && "{{cpu_environment}}/bin/maturin" build --release --locked \
        --interpreter "{{cpu_python}}" --out dist

# Licenses, advisories, sources, bans and single-owner wrappers (deny.toml).
deny:
    #!/usr/bin/env bash
    set -euo pipefail
    # Compare the whole version line, so a different cargo-deny (0.19.0, or 0.20.20) is refused, not just a prefix.
    # `|| true` so a machine without cargo-deny gets the install line below instead of "no such command".
    installed_version="$(cargo deny --version 2>/dev/null || true)"
    if [ "$installed_version" != "cargo-deny {{cargo_deny_version}}" ]; then
        echo "cargo-deny {{cargo_deny_version}} is required, but found: ${installed_version:-no cargo-deny}"
        echo "Install it with: cargo install --locked cargo-deny@{{cargo_deny_version}}"
        exit 1
    fi
    # --allow unused-wrapper: most owner crates in deny.toml don't exist yet; their wrappers are written ahead.
    cargo deny --locked check --allow unused-wrapper

# Nightly CI runs each target for 600 seconds. Crashes land in crates/<crate>/fuzz/artifacts/ (git-ignored).
# Runs one fuzz target of a crate's cargo-fuzz workspace (contracts 12.5) for `seconds`, on the pinned nightly.
fuzz crate target seconds="60":
    #!/usr/bin/env bash
    set -euo pipefail
    installed_version="$(cargo fuzz --version 2>/dev/null || true)"
    if [ "$installed_version" != "cargo-fuzz {{cargo_fuzz_version}}" ]; then
        echo "cargo-fuzz {{cargo_fuzz_version}} is required, but found: ${installed_version:-no cargo-fuzz}"
        echo "Install it with: cargo install --locked cargo-fuzz@{{cargo_fuzz_version}}"
        exit 1
    fi
    # cargo-fuzz finds the fuzz/ folder from the crate's root. fuzz/Cargo.lock is checked in, so the targets build
    # against the same dependency versions every night; `check-fuzz-locks` keeps it current, so a run never rewrites it.
    cd "crates/{{crate}}"
    cargo +{{fuzz_toolchain}} fuzz run "{{target}}" -- -max_total_time="{{seconds}}"

# ---- Repo checks (layout rust-control-plane.md 2.1 and 2.3). Each is one script in tools/checks. ----

# It reads pnpm's installed packages for npm licenses, so it needs env-js; cargo metadata needs no network.
# Every crate, Python package and npm package against its allowlist (Cargo.toml metadata, deps/allowlist-*.json).
check-deps: env-cpu env-js
    "{{check_python}}" tools/checks/check-deps

# No Rust `unsafe` outside the files an ADR names: today only crates/rastai-store/src/disk_space.rs (ADR-0033).
check-unsafe: env-cpu
    "{{check_python}}" tools/checks/check-unsafe

# No pickle, torch.load, torch.save, torch.distributed.checkpoint or pickle-format weights (contracts 12.3).
check-no-pickle: env-cpu
    "{{check_python}}" tools/checks/check-no-pickle

# Python and JavaScript layering, public-API rules and the one wrapper file per library (contracts 2.4, 2.5).
check-imports: env-cpu
    "{{check_python}}" tools/checks/check-imports

# No color literal in frontend code outside @rastai/ui's tokens (contracts 5.6).
check-colors: env-cpu
    "{{check_python}}" tools/checks/check-colors

# No .ts, .tsx or .d.ts file outside the generated folders (ADR-0036).
check-no-typescript: env-cpu
    "{{check_python}}" tools/checks/check-no-typescript

# The root workspace's --locked builds never read the fuzz workspaces' own locks, so without this a dependency change
# leaves them stale and the next `just fuzz` rewrites them. cargo resolves each one; the first run on a new machine
# may fetch the registry index, like `build`.
# Every crates/*/fuzz/Cargo.lock matches its manifests: cargo resolves each fuzz workspace with --locked.
check-fuzz-locks: env-cpu
    "{{check_python}}" tools/checks/check-fuzz-locks

# It reads the checked-in schemas/names.json, which codegen-check keeps current; run `just codegen` after changing a
# name in code. The rules and the places it reads in contracts.md are contracts 12.6.
# Every name in the code is registered in contracts.md, and every registered name is built or carries its milestone.
check-names: env-cpu
    "{{check_python}}" tools/checks/check-names

# It reads the merge base with `git show`, so it needs the base branch's history but no network. It reads the
# checked-in schemas, which codegen-check keeps current; run `just codegen` after changing a command or event type.
# The rules are contracts 12.6.
# Every change to schemas/commands/ or schemas/events/ comes with a contracts.md change, and no stable one breaks.
check-schema-diff base=base_branch: env-cpu
    RASTAI_BASE_REF="{{base}}" "{{check_python}}" tools/checks/check-schema-diff

# ---- Code generation (layout rust-control-plane.md 2.5; rust-node-gateway-proto.md 3.4) ----

# buf.gen.yaml runs the Rust plugins from your PATH, grpcio-tools from the cpu environment and protoc-gen-es from
# node_modules, so it needs both environments. The rastai-codegen bins run after buf, in the order of layout
# rust-control-plane.md 4.9: the catalog first, because every other crate's build needs rastai-diag's tables, then
# the formats generator, then the names registry last, because it reads the catalog, the migrations and the types.
# Regenerate every generated output: the protobuf code of proto/, the error and term catalog of catalog/, the
# schemas, JavaScript types, typed command wrappers, Python dataclasses and commands reference of rastai-formats, and
# schemas/names.json.
codegen: env-cpu env-js proto-tools
    buf generate
    cargo run --quiet --locked -p rastai-codegen --bin rastai-catalog-codegen
    cargo run --quiet --locked -p rastai-codegen --bin rastai-formats-codegen
    cargo run --quiet --locked -p rastai-codegen --bin rastai-names-codegen

# It runs the same generators as `codegen`, so it needs the same tools and both environments. Checking the tool
# versions first means a wrong plugin shows up as an install hint, not as generated code that differs.
# The second line runs rastai-codegen's #[ignore]d tests, the ones a plain `cargo test` on a CI runner can't: the
# catalog's merge-base check (a code present on `base` is never deleted; it needs git history, like proto-check)
# and the outputs checks (generated JavaScript passes tsc and generated Python imports and round-trips; they need both
# environments).
# Checked-in generated code equals a fresh codegen run, and the generators' outputs work in their languages.
codegen-check base=base_branch: env-cpu env-js proto-tools
    "{{check_python}}" tools/checks/check-generated
    RASTAI_BASE_REF="{{base}}" cargo test --quiet --locked -p rastai-codegen -- --ignored

# buf and the Rust protoc plugins at the pinned versions; `codegen` and `codegen-check` run them all.
[private]
proto-tools: (require-version "buf" buf_version) (require-version "protoc-gen-prost" protoc_gen_prost_version) \
    (require-version "protoc-gen-tonic" protoc_gen_tonic_version)

# The one version check every proto recipe uses. Compare whole version lines, so 0.5.0 never passes for 0.5.01.
# `|| true` turns a missing tool into the install hint below instead of "command not found".
[private]
require-version tool version:
    #!/usr/bin/env bash
    set -euo pipefail
    installed="$("{{tool}}" --version 2>/dev/null || true)"
    if [ "$installed" != "{{version}}" ]; then
        echo "{{tool}} {{version}} is required, but found: ${installed:-nothing}."
        echo "The justfile header says how to install it."
        exit 1
    fi

# buf breaking can't see inside arguments_json and the other _json strings; check-schema-diff covers those (M1-58).
# It needs only buf, not the plugins, so it checks buf's version alone.
# buf lint, and buf breaking against the proto files where this branch left `base` (contracts 4).
proto-check base=base_branch: (require-version "buf" buf_version)
    #!/usr/bin/env bash
    set -euo pipefail
    buf lint
    # The merge base, not the tip of base: a field main added after this branch started isn't a removal here.
    merge_base="$(git merge-base HEAD "{{base}}")"
    if ! git cat-file -e "$merge_base:buf.yaml" 2>/dev/null; then
        echo "proto-check: buf lint passed; $merge_base has no buf.yaml, so there is nothing to compare against yet"
        exit 0
    fi
    base_folder="$(mktemp -d)"
    trap 'rm -rf "$base_folder"' EXIT
    git archive "$merge_base" buf.yaml proto | tar -x -C "$base_folder"
    buf breaking --against "$base_folder"
    echo "proto-check: buf lint passed; no breaking change against $merge_base"

# CI's `dco` job passes the pull request's range; locally it checks the commits not yet on origin/main.
# Every commit in the range carries a Signed-off-by matching its author (CONTRIBUTING.md).
check-dco range="origin/main..HEAD": env-cpu
    "{{check_python}}" tools/checks/check-dco "{{range}}"
