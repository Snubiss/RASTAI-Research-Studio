//! What a job may ask the node for, read once from its `JobSpec` when the job is admitted (layout:
//! rust-node-gateway-proto.md sections 7.1 and 7.9). `WorkerHostService` maps a call's job token to its job, then
//! checks the call against these permissions before the owning module runs it.
//!
//! In M1 the only `WorkerHostService` call is `Register`, which needs nothing beyond the token. The checks below are
//! the ones the later calls make: a sandbox (M5), a network host (the egress proxy, M7) and a `GetSecret` purpose
//! (`hf_download` with M2-12, `mysql:<connection_name>` with M5-E07). Each task that adds a call adds its purposes
//! to `secret_purposes_for` and calls its check here; until then no job may ask for a secret.

use rastai_proto::node::v1::JobSpec;

/// What one job may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobPermissions {
    /// The only hosts the job may reach (`JobSpec.network_hosts`).
    pub network_hosts: Vec<String>,
    /// How many sandboxes the job may hold at once (`ResourceRequest.sandbox_count`).
    pub sandbox_count: u32,
    /// The plugin the job belongs to; its commands run as that plugin's principal. Empty for a core job.
    pub plugin_id: String,
    /// The `GetSecret` purposes the job may ask for.
    pub secret_purposes: Vec<String>,
}

/// The permissions a job gets from its spec.
pub fn job_permissions(spec: &JobSpec) -> JobPermissions {
    let sandbox_count = spec.resources.as_ref().map_or(0, |resources| resources.sandbox_count);
    JobPermissions {
        network_hosts: spec.network_hosts.clone(),
        sandbox_count,
        plugin_id: spec.plugin_id.clone(),
        secret_purposes: secret_purposes_for(spec),
    }
}

/// Whether the job may reach `host`. Hosts are compared without regard to case, as DNS does.
pub fn may_reach_host(permissions: &JobPermissions, host: &str) -> bool {
    permissions
        .network_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
}

/// Whether the job may hold one more sandbox while it holds `sandboxes_held`.
pub fn may_create_sandbox(permissions: &JobPermissions, sandboxes_held: u32) -> bool {
    sandboxes_held < permissions.sandbox_count
}

/// Whether the job may ask `GetSecret` for `purpose`.
pub fn may_ask_for_secret(permissions: &JobPermissions, purpose: &str) -> bool {
    permissions.secret_purposes.iter().any(|allowed| allowed == purpose)
}

/// No job kind asks for a secret yet (see the top of this file).
fn secret_purposes_for(_spec: &JobSpec) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_proto::common::v1::ResourceRequest;

    fn spec() -> JobSpec {
        JobSpec {
            network_hosts: vec!["db.lab.internal".to_string()],
            resources: Some(ResourceRequest {
                sandbox_count: 2,
                ..ResourceRequest::default()
            }),
            plugin_id: "evaluation".to_string(),
            ..JobSpec::default()
        }
    }

    #[test]
    fn a_job_may_use_only_what_its_spec_names() {
        let permissions = job_permissions(&spec());
        assert!(may_reach_host(&permissions, "db.lab.internal"));
        assert!(may_reach_host(&permissions, "DB.Lab.Internal"));
        assert!(!may_reach_host(&permissions, "example.com"));
        assert!(may_create_sandbox(&permissions, 1));
        assert!(!may_create_sandbox(&permissions, 2));
        assert_eq!(permissions.plugin_id, "evaluation");
    }

    #[test]
    fn no_job_may_ask_for_a_secret_yet_and_a_bare_spec_may_use_nothing() {
        assert!(!may_ask_for_secret(&job_permissions(&spec()), "hf_download"));
        let bare = job_permissions(&JobSpec::default());
        assert!(!may_reach_host(&bare, "localhost"));
        assert!(!may_create_sandbox(&bare, 0));
    }
}
