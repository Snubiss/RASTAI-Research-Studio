//! rastai-diag: the `Diagnostic` type every crate returns, the compiled catalog, the renderers and the
//! JSON-lines logger (layout: rust-control-plane.md section 3).
//!
//! This file is a list of `pub mod` lines (contracts 12.2), plus the M1-03 test that the crate inherits the
//! workspace package fields.

pub mod catalog;
pub mod diagnostic;
pub mod generated;
pub mod logger;
pub mod path;
pub mod render_basic;
pub mod render_json;
pub mod render_text;
pub mod terms;

#[cfg(test)]
mod tests {
    // The values every crate must inherit from [workspace.package] in the root Cargo.toml.
    // A crate that stops inheriting them drifts from the project's license and toolchain.
    // The version is not checked: release-please bumps it on every release, and a hardcoded copy here would fail
    // `just check` on each bump. The license and rust-version checks already show the crate reads [workspace.package].
    // WORKSPACE_RUST_VERSION changes in the same PR as rust-toolchain.toml and [workspace.package] rust-version.
    const WORKSPACE_LICENSE: &str = "Apache-2.0";
    const WORKSPACE_RUST_VERSION: &str = "1.99";

    // Cargo fills these from the package fields at compile time, so they show what the crate inherited.
    #[test]
    fn package_fields_come_from_the_workspace() {
        assert_eq!(env!("CARGO_PKG_LICENSE"), WORKSPACE_LICENSE);
        assert_eq!(env!("CARGO_PKG_RUST_VERSION"), WORKSPACE_RUST_VERSION);
    }
}
