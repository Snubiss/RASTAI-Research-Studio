//! The tonic `WorkerHostService` on `worker-host.sock` (contracts 4.5; layout: rust-node-gateway-proto.md section
//! 7.1): every call maps its job token to its job, checks the call against the job's permissions, then hands it to
//! the module that owns it. In M1 the service has one call, `Register`, which jobs/supervisor.rs answers.
//!
//! The job token travels in the gRPC metadata key `rastai-job-token`, as the 64 hex characters of
//! `RASTAI_JOB_TOKEN`, both ways: the worker sends it here, and the node sends it on its `WorkerService` calls
//! (contracts 3.2 and 4.5). A refusal comes back as a `Diagnostic` in the response with gRPC status OK, because the
//! call itself ran (contracts 4).

use std::path::Path;
use std::sync::Arc;

use rastai_core::paths::worker_host_socket_path;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::diagnostic;
use rastai_diag::logger::log_error;
use rastai_proto::worker::v1::worker_host_service_server::{WorkerHostService, WorkerHostServiceServer};
use rastai_proto::worker::v1::{RegisterRequest, RegisterResponse};
use rastai_transport::grpc_channel::serve_grpc_on_socket;
use rastai_transport::local_socket::bind_local_socket;
use tokio::task::JoinHandle;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::error::NodeError;
use crate::jobs::supervisor::register_worker;
use crate::node::NodeState;

/// The gRPC metadata key of the job token (contracts 4.5).
pub const JOB_TOKEN_METADATA_KEY: &str = "rastai-job-token";

/// The served WorkerHostService. Serving stops when this is dropped, like the telemetry loops.
pub struct WorkerHostServer {
    handle: JoinHandle<()>,
}

impl Drop for WorkerHostServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// The WorkerHostService server's handle on the node's state.
pub(crate) struct WorkerHostHandler {
    state: Arc<NodeState>,
}

#[tonic::async_trait]
impl WorkerHostService for WorkerHostHandler {
    async fn register(&self, request: Request<RegisterRequest>) -> Result<Response<RegisterResponse>, Status> {
        let job_token = job_token_from_metadata(request.metadata());
        let message = request.into_inner();
        let refusal = match job_token {
            Some(job_token) => {
                let socket_path = Path::new(&message.job_socket_path);
                register_worker(&self.state, &message.job_id, &job_token, socket_path)
                    .await
                    .err()
            }
            None => Some(diagnostic("RRS-J2002", &[("job_id", &message.job_id)])),
        };
        let response = RegisterResponse {
            diagnostic: refusal.as_ref().map(diagnostic_to_proto),
        };
        Ok(Response::new(response))
    }
}

/// Binds `worker-host.sock` and serves WorkerHostService on it from a task of its own. The node starts it before
/// anything else that involves jobs, so a live job can re-register as soon as the node is back (layout 7.3).
pub fn serve_worker_host(state: &Arc<NodeState>) -> Result<WorkerHostServer, NodeError> {
    let listener = bind_local_socket(&worker_host_socket_path(&state.paths))?;
    let handler = WorkerHostHandler {
        state: Arc::clone(state),
    };
    let router = Server::builder().add_service(WorkerHostServiceServer::new(handler));
    let handle = tokio::spawn(async move {
        if let Err(error) = serve_grpc_on_socket(listener, router).await {
            let reason = format!("{error:?}");
            log_error("worker-host.sock stopped serving", &[("reason", &reason)]);
        }
    });
    Ok(WorkerHostServer { handle })
}

/// The job token a call carries, if it carries one that is text.
pub fn job_token_from_metadata(metadata: &MetadataMap) -> Option<String> {
    let value = metadata.get(JOB_TOKEN_METADATA_KEY)?;
    value.to_str().ok().map(str::to_string)
}

/// A request that carries the job token in its metadata. The error says why the token can't be a metadata value.
pub fn request_with_job_token<T>(message: T, job_token: &str) -> Result<Request<T>, String> {
    let value = MetadataValue::try_from(job_token).map_err(|_| "the job token isn't plain text".to_string())?;
    let mut request = Request::new(message);
    request.metadata_mut().insert(JOB_TOKEN_METADATA_KEY, value);
    Ok(request)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_token_travels_in_its_metadata_key() {
        let token = "0f".repeat(32);
        let request = request_with_job_token((), &token).unwrap();
        assert_eq!(job_token_from_metadata(request.metadata()), Some(token));
        assert_eq!(job_token_from_metadata(&MetadataMap::new()), None);
        assert!(request_with_job_token((), "line\nbreak").is_err());
    }
}
