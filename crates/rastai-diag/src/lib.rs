//! rastai-diag: the `Diagnostic` type every crate returns, the compiled catalog, the renderers and the
//! JSON-lines logger (layout: rust-control-plane.md section 3).
//!
//! This file is a list of `pub mod` lines. The modules arrive with M1-11; until then the crate is an empty
//! member that proves the workspace builds and that its shared settings reach every crate.

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
