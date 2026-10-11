//! The tonic server stubs build from the generated code (layout: rust-node-gateway-proto.md sections 3.4 and 3.6).
//!
//! One stub per gRPC service implements the generated server trait, and each is wrapped in its generated server.
//! The trait has one method per RPC, so this file stops compiling when an RPC is added, removed or changes its
//! messages or streaming, and the gRPC names checked below are the ones the Python and JavaScript sides call.
//! The stubs answer every call with `unimplemented`; rastai-node and rastai-server write the real handlers.

use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use rastai_proto::events::v1::EventRecord;
use rastai_proto::node::v1::node_hub_service_server::{NodeHubService, NodeHubServiceServer};
use rastai_proto::node::v1::node_service_server::{NodeService, NodeServiceServer};
use rastai_proto::node::v1::{
    GetHardwareRequest, GetHardwareResponse, HandshakeRequest, HandshakeResponse, HeartbeatRequest, HeartbeatResponse,
    JobInfo, JobSpec, ListJobsRequest, ListJobsResponse, LiveEvent, SignalJobRequest, SignalJobResponse,
    WatchLiveRequest, WatchRunEventsRequest,
};
use rastai_proto::worker::v1::worker_host_service_server::{WorkerHostService, WorkerHostServiceServer};
use rastai_proto::worker::v1::worker_service_server::{WorkerService, WorkerServiceServer};
use rastai_proto::worker::v1::{
    ControlRequest, ControlResponse, HelloRequest, RegisterRequest, RegisterResponse, WorkerInfo,
};
use tonic::codegen::tokio_stream::{self, Empty};
use tonic::server::NamedService;
use tonic::{Request, Response, Status};

// The full protobuf names of the services, which are also the gRPC path prefixes ("/<name>/<Rpc>").
const NODE_SERVICE_NAME: &str = "rastai.node.v1.NodeService";
const NODE_HUB_SERVICE_NAME: &str = "rastai.node.v1.NodeHubService";
const WORKER_SERVICE_NAME: &str = "rastai.worker.v1.WorkerService";
const WORKER_HOST_SERVICE_NAME: &str = "rastai.worker.v1.WorkerHostService";

/// A stream with no items, for the server-streaming RPCs of a stub.
type NoItems<T> = Empty<Result<T, Status>>;

/// The answer every stub method gives.
fn not_in_this_stub<T>() -> Result<Response<T>, Status> {
    Err(Status::unimplemented("a stub built only to check the generated code"))
}

struct StubNode;

#[tonic::async_trait]
impl NodeService for StubNode {
    async fn handshake(&self, _request: Request<HandshakeRequest>) -> Result<Response<HandshakeResponse>, Status> {
        not_in_this_stub()
    }

    async fn get_hardware(
        &self,
        _request: Request<GetHardwareRequest>,
    ) -> Result<Response<GetHardwareResponse>, Status> {
        not_in_this_stub()
    }

    async fn start_job(&self, _request: Request<JobSpec>) -> Result<Response<JobInfo>, Status> {
        not_in_this_stub()
    }

    async fn signal_job(&self, _request: Request<SignalJobRequest>) -> Result<Response<SignalJobResponse>, Status> {
        not_in_this_stub()
    }

    async fn list_jobs(&self, _request: Request<ListJobsRequest>) -> Result<Response<ListJobsResponse>, Status> {
        not_in_this_stub()
    }

    type WatchRunEventsStream = NoItems<EventRecord>;

    async fn watch_run_events(
        &self,
        _request: Request<WatchRunEventsRequest>,
    ) -> Result<Response<Self::WatchRunEventsStream>, Status> {
        Ok(Response::new(tokio_stream::empty()))
    }

    type WatchLiveStream = NoItems<LiveEvent>;

    async fn watch_live(&self, _request: Request<WatchLiveRequest>) -> Result<Response<Self::WatchLiveStream>, Status> {
        Ok(Response::new(tokio_stream::empty()))
    }
}

struct StubNodeHub;

#[tonic::async_trait]
impl NodeHubService for StubNodeHub {
    async fn heartbeat(&self, _request: Request<HeartbeatRequest>) -> Result<Response<HeartbeatResponse>, Status> {
        not_in_this_stub()
    }

    async fn execute_command(&self, _request: Request<ExecuteRequest>) -> Result<Response<ExecuteResponse>, Status> {
        not_in_this_stub()
    }
}

struct StubWorker;

#[tonic::async_trait]
impl WorkerService for StubWorker {
    async fn hello(&self, _request: Request<HelloRequest>) -> Result<Response<WorkerInfo>, Status> {
        not_in_this_stub()
    }

    async fn control(&self, _request: Request<ControlRequest>) -> Result<Response<ControlResponse>, Status> {
        not_in_this_stub()
    }

    type WatchLiveStream = NoItems<LiveEvent>;

    async fn watch_live(&self, _request: Request<WatchLiveRequest>) -> Result<Response<Self::WatchLiveStream>, Status> {
        Ok(Response::new(tokio_stream::empty()))
    }
}

struct StubWorkerHost;

#[tonic::async_trait]
impl WorkerHostService for StubWorkerHost {
    async fn register(&self, _request: Request<RegisterRequest>) -> Result<Response<RegisterResponse>, Status> {
        not_in_this_stub()
    }
}

/// The gRPC service name of a generated server, as tonic routes calls to it.
fn served_name<S: NamedService>(_server: &S) -> &'static str {
    S::NAME
}

#[test]
fn every_server_stub_builds_under_its_proto_name() {
    let served_names = [
        served_name(&NodeServiceServer::new(StubNode)),
        served_name(&NodeHubServiceServer::new(StubNodeHub)),
        served_name(&WorkerServiceServer::new(StubWorker)),
        served_name(&WorkerHostServiceServer::new(StubWorkerHost)),
    ];
    let proto_names = [
        NODE_SERVICE_NAME,
        NODE_HUB_SERVICE_NAME,
        WORKER_SERVICE_NAME,
        WORKER_HOST_SERVICE_NAME,
    ];
    assert_eq!(served_names, proto_names);
}
