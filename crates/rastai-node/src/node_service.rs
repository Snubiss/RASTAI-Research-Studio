//! The tonic `NodeService` (contracts 4.4): one short function per RPC, each calling the module that owns it
//! (layout: rust-node-gateway-proto.md section 7.1). A refusal comes back as a `Diagnostic` in the response with
//! gRPC status OK, because the call itself ran (contracts 4).
//!
//! The event RPCs answer UNIMPLEMENTED until their task lands: WatchRunEvents and WatchLive with M1-44. Nothing
//! calls them before then.

use std::sync::Arc;

use rastai_proto::events::v1::EventRecord;
use rastai_proto::node::v1::node_service_server::NodeService;
use rastai_proto::node::v1::{
    GetHardwareRequest, GetHardwareResponse, HandshakeRequest, HandshakeResponse, JobInfo, JobSpec, ListJobsRequest,
    ListJobsResponse, LiveEvent, SignalJobRequest, SignalJobResponse, WatchLiveRequest, WatchRunEventsRequest,
};
use tonic::codegen::tokio_stream;
use tonic::{Request, Response, Status};

use crate::handshake::answer_handshake;
use crate::jobs::supervisor::{list_jobs, signal_job, start_job};
use crate::node::{NodeState, current_hardware, current_telemetry};

const STREAMS_ARRIVE_WITH: &str = "run events and live streams arrive with M1-44";

/// The NodeService server's handle on the node's state.
pub(crate) struct NodeServiceHandler {
    state: Arc<NodeState>,
}

impl NodeServiceHandler {
    pub(crate) fn new(state: Arc<NodeState>) -> NodeServiceHandler {
        NodeServiceHandler { state }
    }
}

#[tonic::async_trait]
impl NodeService for NodeServiceHandler {
    async fn handshake(&self, request: Request<HandshakeRequest>) -> Result<Response<HandshakeResponse>, Status> {
        Ok(Response::new(answer_handshake(&self.state, request.get_ref())))
    }

    async fn get_hardware(
        &self,
        _request: Request<GetHardwareRequest>,
    ) -> Result<Response<GetHardwareResponse>, Status> {
        let response = GetHardwareResponse {
            hardware: Some(current_hardware(&self.state)),
            telemetry: Some(current_telemetry(&self.state)),
        };
        Ok(Response::new(response))
    }

    async fn start_job(&self, request: Request<JobSpec>) -> Result<Response<JobInfo>, Status> {
        Ok(Response::new(start_job(&self.state, request.into_inner()).await))
    }

    async fn signal_job(&self, request: Request<SignalJobRequest>) -> Result<Response<SignalJobResponse>, Status> {
        let message = request.into_inner();
        let diagnostic = signal_job(&self.state, &message.job_id, message.signal()).await;
        Ok(Response::new(SignalJobResponse { diagnostic }))
    }

    async fn list_jobs(&self, _request: Request<ListJobsRequest>) -> Result<Response<ListJobsResponse>, Status> {
        let jobs = list_jobs(&self.state);
        Ok(Response::new(ListJobsResponse { jobs }))
    }

    type WatchRunEventsStream = tokio_stream::Empty<Result<EventRecord, Status>>;

    async fn watch_run_events(
        &self,
        _request: Request<WatchRunEventsRequest>,
    ) -> Result<Response<Self::WatchRunEventsStream>, Status> {
        Err(Status::unimplemented(STREAMS_ARRIVE_WITH))
    }

    type WatchLiveStream = tokio_stream::Empty<Result<LiveEvent, Status>>;

    async fn watch_live(&self, _request: Request<WatchLiveRequest>) -> Result<Response<Self::WatchLiveStream>, Status> {
        Err(Status::unimplemented(STREAMS_ARRIVE_WITH))
    }
}
