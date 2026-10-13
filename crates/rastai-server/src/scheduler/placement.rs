//! Which node a job runs on (layout: rust-control-plane.md 8.1; not layer placement, which is
//! modelspec_analysis/layer_placement.rs). M1's placement is trivial: every job goes to the node `compute.default_node`
//! names, which is `local`, the embedded node, by default. GPU choice from telemetry and leases joins with them
//! (M2, M3); until then a run trains on the CPU (handlers/train.rs).

use rastai_core::settings::setting_text;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};

use crate::nodes::node_links::{embedded_node_id, link_is_connected, node_is_known};
use crate::server_state::Server;

const DEFAULT_NODE_SETTING: &str = "compute.default_node";
/// The value of `compute.default_node` that means the embedded node (contracts 11.2).
const LOCAL_NODE: &str = "local";
const CODE_NO_SUCH_NODE: &str = "RRS-N0008";

/// The node a new job goes to. A job may be queued for a node that isn't connected right now; it starts when the
/// node is back. A node the daemon doesn't know is `RRS-N0008`.
pub fn pick_node(server: &Server) -> Result<String, Vec<Diagnostic>> {
    let wanted = default_node_setting(server);
    let node_id = if wanted == LOCAL_NODE || wanted.is_empty() {
        embedded_node_id(server)
    } else {
        wanted
    };
    if node_is_known(server, &node_id) {
        return Ok(node_id);
    }
    Err(vec![diagnostic(CODE_NO_SUCH_NODE, &[("node_id", &node_id)])])
}

/// True when the node is connected, so a queued job can be launched on it now.
pub fn node_is_ready(server: &Server, node_id: &str) -> bool {
    link_is_connected(server, node_id)
}

fn default_node_setting(server: &Server) -> String {
    let settings = match server.settings.read() {
        Ok(settings) => settings,
        // A panic while the settings were replaced leaves the old or the new value, either whole.
        Err(poisoned) => poisoned.into_inner(),
    };
    setting_text(&settings, DEFAULT_NODE_SETTING)
}
