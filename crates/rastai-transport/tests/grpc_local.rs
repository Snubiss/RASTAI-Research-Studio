//! A tiny tonic service over a Unix socket, unary and streaming (layout: rust-node-gateway-proto.md section 4.7).
//! The service is the generated WorkerService, the smallest one with both kinds of call: Hello is unary and
//! WatchLive streams. The local NodeConnection is checked on the same server, since a channel is a channel.
//!
//! Unix sockets only; on Windows the functions return RRS-N2009 until named pipes arrive (tier 2).

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod on_unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use rastai_core::paths::DataPaths;
    use rastai_proto::node::v1::{LiveEvent, WatchLiveRequest};
    use rastai_proto::worker::v1::worker_service_client::WorkerServiceClient;
    use rastai_proto::worker::v1::worker_service_server::{WorkerService, WorkerServiceServer};
    use rastai_proto::worker::v1::{ControlRequest, ControlResponse, HelloRequest, WorkerInfo};
    use rastai_transport::grpc_channel::{connect_grpc_channel, serve_grpc_on_socket};
    use rastai_transport::local_socket::bind_local_socket;
    use rastai_transport::node_connection::{NodeTarget, close_node_connection, node_channel, open_node_connection};
    use tonic::codegen::tokio_stream;
    use tonic::{Request, Response, Status};

    use crate::common::{SOCKET_FOLDER, new_test_folder, socket_path_in};

    const SOCKET_FILE: &str = "worker.sock";
    const TEST_FOLDER_PREFIX: &str = "rtg";
    /// A socket folder others can enter, which the client refuses (contracts 8.1).
    const OPEN_FOLDER_MODE: u32 = 0o755;
    const TINY_JOB_ID: &str = "job_tiny";
    const TINY_PROCESS_ID: u32 = 4242;
    /// How many events WatchLive sends before it ends.
    const STREAMED_EVENT_COUNT: i64 = 5;

    /// Answers Hello with fixed values and streams STREAMED_EVENT_COUNT numbered events from WatchLive.
    struct TinyWorker;

    #[tonic::async_trait]
    impl WorkerService for TinyWorker {
        async fn hello(&self, _request: Request<HelloRequest>) -> Result<Response<WorkerInfo>, Status> {
            let info = WorkerInfo {
                job_id: TINY_JOB_ID.to_string(),
                pid: TINY_PROCESS_ID,
                ..WorkerInfo::default()
            };
            Ok(Response::new(info))
        }

        async fn control(&self, _request: Request<ControlRequest>) -> Result<Response<ControlResponse>, Status> {
            Err(Status::unimplemented("the tiny worker has no controls"))
        }

        type WatchLiveStream = tokio_stream::Iter<std::vec::IntoIter<Result<LiveEvent, Status>>>;

        async fn watch_live(
            &self,
            _request: Request<WatchLiveRequest>,
        ) -> Result<Response<Self::WatchLiveStream>, Status> {
            let mut events = Vec::new();
            for number in 0..STREAMED_EVENT_COUNT {
                let event = LiveEvent {
                    job_id: TINY_JOB_ID.to_string(),
                    emitted_unix_ms: number,
                    ..LiveEvent::default()
                };
                events.push(Ok(event));
            }
            Ok(Response::new(tokio_stream::iter(events)))
        }
    }

    fn new_socket_path() -> (PathBuf, PathBuf) {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder, SOCKET_FILE);
        (test_folder, socket_path)
    }

    /// Binds the socket and serves the tiny worker on it in the background.
    fn start_tiny_server(socket_path: &Path) {
        let listener = bind_local_socket(socket_path).unwrap();
        let router = tonic::transport::Server::builder().add_service(WorkerServiceServer::new(TinyWorker));
        tokio::spawn(serve_grpc_on_socket(listener, router));
    }

    #[tokio::test]
    async fn a_unary_call_round_trips_over_a_unix_socket() {
        let (test_folder, socket_path) = new_socket_path();
        start_tiny_server(&socket_path);

        let channel = connect_grpc_channel(&socket_path).await.unwrap();
        let mut client = WorkerServiceClient::new(channel);
        let info = client.hello(HelloRequest {}).await.unwrap().into_inner();

        assert_eq!(info.job_id, TINY_JOB_ID);
        assert_eq!(info.pid, TINY_PROCESS_ID);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_server_stream_arrives_whole_and_in_order_over_a_unix_socket() {
        let (test_folder, socket_path) = new_socket_path();
        start_tiny_server(&socket_path);

        let channel = connect_grpc_channel(&socket_path).await.unwrap();
        let mut client = WorkerServiceClient::new(channel);
        let mut stream = client.watch_live(WatchLiveRequest {}).await.unwrap().into_inner();
        let mut received = Vec::new();
        while let Some(event) = stream.message().await.unwrap() {
            received.push(event.emitted_unix_ms);
        }

        assert_eq!(received, (0..STREAMED_EVENT_COUNT).collect::<Vec<i64>>());
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_status_error_crosses_the_socket_as_a_status() {
        let (test_folder, socket_path) = new_socket_path();
        start_tiny_server(&socket_path);

        let channel = connect_grpc_channel(&socket_path).await.unwrap();
        let mut client = WorkerServiceClient::new(channel);
        let status = client.control(ControlRequest::default()).await.unwrap_err();

        assert_eq!(status.code(), tonic::Code::Unimplemented);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn connecting_with_nothing_listening_is_not_reachable() {
        let (test_folder, socket_path) = new_socket_path();
        let refusal = connect_grpc_channel(&socket_path).await.map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2004"));

        // The folder exists and is private now, but nobody serves the socket.
        drop(bind_local_socket(&socket_path).unwrap());
        let refusal = connect_grpc_channel(&socket_path).await.map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2004"));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_socket_folder_others_can_enter_is_refused_before_connecting() {
        // tonic's unix: endpoint reads no peer credentials, so this folder check is what proves the server is ours.
        let (test_folder, socket_path) = new_socket_path();
        start_tiny_server(&socket_path);
        let socket_folder = test_folder.join(SOCKET_FOLDER);
        fs::set_permissions(&socket_folder, fs::Permissions::from_mode(OPEN_FOLDER_MODE)).unwrap();

        let refusal = connect_grpc_channel(&socket_path).await.map(|_| ());

        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_local_node_connection_carries_calls() {
        let (test_folder, socket_path) = new_socket_path();
        start_tiny_server(&socket_path);
        let paths = DataPaths {
            config: test_folder.join("config"),
            data: test_folder.join("data"),
            cache: test_folder.join("cache"),
            state: test_folder.join("state"),
            runtime: test_folder.join(SOCKET_FOLDER),
        };
        let target = NodeTarget::Local {
            socket_path: socket_path.clone(),
        };

        let connection = open_node_connection(&target, &paths).await.unwrap();
        let mut client = WorkerServiceClient::new(node_channel(&connection));
        let info = client.hello(HelloRequest {}).await.unwrap().into_inner();
        close_node_connection(connection).await;

        assert_eq!(info.job_id, TINY_JOB_ID);
        fs::remove_dir_all(&test_folder).unwrap();
    }
}

#[cfg(windows)]
mod on_windows {
    use std::path::Path;

    use rastai_transport::grpc_channel::connect_grpc_channel;

    #[tokio::test]
    async fn grpc_channels_say_they_arent_supported_yet() {
        let refusal = connect_grpc_channel(Path::new(r"C:\rastai-test\node.sock"))
            .await
            .map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2009"));
    }
}
