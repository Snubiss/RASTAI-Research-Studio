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
# Later tasks add their recipes here: the Python and JavaScript ones (M1-04), the repo checks (M1-05),
# codegen (M1-12), proto (M1-08), check-names (M1-57) and check-schema-diff (M1-58).

# `deny` refuses any other cargo-deny version, so every machine reads deny.toml the same way.
cargo_deny_version := "0.20.2"
# `fuzz` runs on this exact nightly and cargo-fuzz, so a crash found one night reproduces the next.
fuzz_toolchain := "nightly-2026-09-27"
cargo_fuzz_version := "0.13.2"

set shell := ["bash", "-euo", "pipefail", "-c"]

# The gate every PR passes: format, lint, tests, the browser build and dependency policy.
check: fmt-check lint test-rust wasm deny

# Build every crate in the workspace. --locked: Cargo.lock is checked in and never silently changes.
build:
    cargo build --workspace --locked

# Unit tests and crates/*/tests/ for the whole workspace.
test-rust:
    cargo test --workspace --locked

# Fails if any file isn't formatted by rustfmt (rustfmt.toml: 120 columns).
fmt-check:
    cargo fmt --all --check

# The rastai-formats-wasm bindings and their wasm-bindgen step join this recipe in M4.
# The crates that also run in the browser must build for wasm32-unknown-unknown (layout sections 3.4 and 4.10).
wasm:
    cargo build -p rastai-diag -p rastai-formats --target wasm32-unknown-unknown --locked

# clippy with the workspace lints (Cargo.toml) and thresholds (clippy.toml); every warning fails.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

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
