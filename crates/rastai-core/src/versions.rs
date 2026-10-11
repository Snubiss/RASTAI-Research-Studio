//! Version constants every process reports and checks (spec §20, layout rust-control-plane.md section 5.2).
//!
//! The app, the node and the worker ship together under one version, the workspace's. The node and worker
//! protocols are plain integers with capability flags beside them in the handshake, so an older peer is told
//! "incompatible" instead of failing on a field it can't read.

/// The app version, from `[workspace.package] version` in the root Cargo.toml (release-please bumps it).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The plugin API version. Frozen as 1.0.0 in Phase 2, once the API is extracted from first-party code (spec §13).
pub const PLUGIN_API_VERSION: &str = "0.1.0";

/// `NodeService.Handshake`'s protocol version (contracts 9.2). A mismatch makes a node `incompatible`.
pub const NODE_PROTOCOL_VERSION: u32 = 1;

/// `WorkerService.Hello`'s protocol version (contracts 9.1). A worker speaking another one is sent `TERMINATE`.
pub const WORKER_PROTOCOL_VERSION: u32 = 1;

/// The proto packages of the daemon's public HTTP API (layout section 11): what a client may depend on.
pub const API_PACKAGES: &[&str] = &["rastai.commands.v1", "rastai.events.v1"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // The proto tree, from this crate's folder.
    const PROTO_ROOT: &str = "../../proto";

    #[test]
    fn app_version_is_three_numbers() {
        let parts: Vec<&str> = APP_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "{APP_VERSION}");
        assert!(
            parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        );
    }

    // A package listed here that the proto tree doesn't have would advertise an API that doesn't exist.
    #[test]
    fn every_api_package_is_a_proto_package() {
        for package in API_PACKAGES {
            let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(PROTO_ROOT)
                .join(package.replace('.', "/"));
            assert!(folder.is_dir(), "{package}: no folder {}", folder.display());
        }
    }
}
