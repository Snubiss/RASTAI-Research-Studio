# Every build, test and check task goes through `just` (layout: rust-control-plane.md section 2.5).
# "Done" means `just check` passes (§29). Running `just` with no recipe runs `check`.
#
# Dev tools, pinned. Install them for your user (no sudo) with:
#   cargo install --locked just@1.58.0
#   cargo install --locked cargo-deny@0.20.2
#   cargo install --locked cargo-fuzz@0.13.2       (only for `just fuzz`)
#   rustup toolchain install nightly-2026-09-27 --profile minimal   (only for `just fuzz`)
# cargo install checks each crate against the checksum in the crates.io index.
# The Rust toolchain is pinned in rust-toolchain.toml; rustup installs it on first use. Fuzzing needs nightly
# (cargo-fuzz builds with sanitizer flags), so its toolchain is pinned here instead: fuzz_toolchain below.
#
# Python and JavaScript tools, also for your user only:
#   uv 0.13.0, which `required-version` in python/pyproject.toml and python/envs/cpu/pyproject.toml enforces.
#     Download https://github.com/astral-sh/uv/releases/download/0.13.0/uv-x86_64-unknown-linux-gnu.tar.gz,
#     check it with `sha256sum`: 1468ebd5a5541121837c5a2817b9972ba6090fa6caa3d142620850a47fb75154,
#     and put uv and uvx in ~/.local/bin. `just env-cpu` then fetches Python 3.12 itself; uv checks that download.
#   Node.js 24 from nodejs.org (check the tarball against its SHASUMS256.txt), then pnpm through Node's corepack:
#     corepack enable --install-directory ~/.local/bin pnpm
#     package.json `packageManager` pins pnpm 12.7.0 with its sha512, and corepack refuses a download that differs.
#
# Later tasks add their recipes here: the repo checks (M1-05), codegen (M1-12), proto (M1-08),
# check-names (M1-57) and check-schema-diff (M1-58).

# `deny` refuses any other cargo-deny version, so every machine reads deny.toml the same way.
cargo_deny_version := "0.20.2"
# `fuzz` runs on this exact nightly and cargo-fuzz, so a crash found one night reproduces the next.
fuzz_toolchain := "nightly-2026-09-27"
cargo_fuzz_version := "0.13.2"
# The cpu environment for development, CI and the M1 embedded node (contracts 15.8; git-ignored).
cpu_environment := justfile_directory() / ".dev/envs/cpu"

set shell := ["bash", "-euo", "pipefail", "-c"]

# The gate every PR passes: format, lint, the JSDoc type check, tests, the browser build and dependency policy.
check: fmt-check lint check-js test-rust test-python test-js wasm deny

# Build every crate in the workspace. --locked: Cargo.lock is checked in and never silently changes.
build:
    cargo build --workspace --locked

# Unit tests and crates/*/tests/ for the whole workspace.
test-rust:
    cargo test --workspace --locked

# Rust by rustfmt (rustfmt.toml), Python by ruff (python/pyproject.toml), JavaScript, JSON, CSS and YAML by
# prettier (.prettierrc.json, .prettierignore). All three use 120 columns.
# Fails if any file isn't formatted.
fmt-check: env-cpu env-js
    cargo fmt --all --check
    "{{cpu_environment}}/bin/ruff" format --check python
    pnpm exec prettier --check .

# The rastai-formats-wasm bindings and their wasm-bindgen step join this recipe in M4.
# The crates that also run in the browser must build for wasm32-unknown-unknown (layout sections 3.4 and 4.10).
wasm:
    cargo build -p rastai-diag -p rastai-formats --target wasm32-unknown-unknown --locked

# Rust: the workspace lints (Cargo.toml) and thresholds (clippy.toml). Python: the rules in python/pyproject.toml.
# clippy and ruff check; every warning fails.
lint: env-cpu
    cargo clippy --workspace --all-targets --locked -- -D warnings
    "{{cpu_environment}}/bin/ruff" check python

# This is the same `uv sync --frozen` the node runs from M2-16 (the node also leaves out the dev group, so pytest
# and ruff never reach a user's machine). --frozen installs exactly the lock and never resolves, so after one
# online run it works offline from the uv cache.
# Sync the cpu environment from python/envs/cpu/uv.lock into .dev/envs/cpu.
env-cpu:
    UV_PROJECT_ENVIRONMENT="{{cpu_environment}}" uv sync --frozen --project python/envs/cpu

# --frozen-lockfile installs exactly the lock and fails instead of changing it.
# Install the JavaScript dev tools (typescript checker, vitest, prettier) from pnpm-lock.yaml into node_modules/.
env-js:
    pnpm install --frozen-lockfile

# Settings and markers are in python/pyproject.toml; python/conftest.py skips gpu tests without a device and
# posix_only tests on Windows.
# pytest over python/ in the cpu environment.
test-python: env-cpu
    cd python && "{{cpu_environment}}/bin/python" -m pytest

# The test files are listed in vitest.config.js.
# vitest over the whole JavaScript workspace.
test-js: env-js
    pnpm exec vitest run

# tsc is a checker only (ADR-0036): --noEmit here, and noEmit in jsconfig.base.json too. jsconfig.json covers the
# whole workspace; each package's own jsconfig.json extends the same base, so an editor and this check agree.
# Type-check the JSDoc in every .js and .jsx file.
check-js: env-js
    pnpm exec tsc --noEmit -p jsconfig.json

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
    # against the same dependency versions every night.
    cd "crates/{{crate}}"
    cargo +{{fuzz_toolchain}} fuzz run "{{target}}" -- -max_total_time="{{seconds}}"
