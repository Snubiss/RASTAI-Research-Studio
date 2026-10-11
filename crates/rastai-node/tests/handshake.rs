//! The handshake over node.sock, as the daemon makes it (node layout 7.9; designer flow C3): a daemon whose
//! protocol version differs gets RRS-N0001, a daemon key the node hasn't pinned gets RRS-N0002, and a pinned daemon
//! gets a session it can check. Then GetHardware and telemetry over the same socket, and a second node on the same
//! machine refused with RRS-N0003.
//!
//! Unix only: the node serves a Unix socket, and the stand-in tools are shell scripts.

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod on_unix {
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use rastai_core::paths::{node_key_path, paired_daemons_path};
    use rastai_core::random::random_bytes;
    use rastai_core::time::now_unix_ms;
    use rastai_core::versions::{APP_VERSION, NODE_PROTOCOL_VERSION};
    use rastai_node::handshake::{daemon_challenge_bytes, node_challenge_bytes, pin_daemon_key};
    use rastai_node::identity::{
        NodeIdentity, daemon_id_from_public_key, load_or_create_identity, public_key_bytes, sign_challenge,
        verify_challenge,
    };
    use rastai_node::node::run_node;
    use rastai_proto::node::v1::node_service_client::NodeServiceClient;
    use rastai_proto::node::v1::{GetHardwareRequest, HandshakeRequest, HandshakeResponse};
    use tonic::transport::Channel;

    use crate::common::{NvidiaSmi, machine_from_fixture, new_test_folder, start_node, test_node_options, test_paths};

    const TEST_FOLDER_PREFIX: &str = "rnhk";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    const HUB_SOCKET: &str = "/run/user/1000/rastai/node-hub.sock";
    /// The daemon's nonce: 32 random bytes, as rastai-server sends.
    const NONCE_BYTES: usize = 32;
    /// How long the telemetry test waits for the stand-in nvidia-smi's first lines to reach GetHardware.
    const TELEMETRY_WAIT: Duration = Duration::from_secs(10);
    const TELEMETRY_POLL: Duration = Duration::from_millis(50);

    /// A test machine with a daemon identity of its own, and the node's id (from the node key made up front).
    struct Machine {
        folder: std::path::PathBuf,
        daemon: NodeIdentity,
        daemon_id: String,
        node_id: String,
    }

    fn machine() -> Machine {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let node = load_or_create_identity(&node_key_path(&test_paths(&folder))).unwrap();
        let daemon = load_or_create_identity(&folder.join("daemon.key")).unwrap();
        Machine {
            daemon_id: daemon_id_from_public_key(&public_key_bytes(&daemon)),
            node_id: rastai_core::ids::node_id_from_public_key(&public_key_bytes(&node)),
            folder,
            daemon,
        }
    }

    /// The daemon's half, signed with its key over the node's id, as rastai-server builds it.
    fn daemon_request(machine: &Machine, protocol_version: u32) -> HandshakeRequest {
        let nonce = random_bytes(NONCE_BYTES).unwrap();
        let sent_unix_ms = now_unix_ms();
        let signed = daemon_challenge_bytes(&machine.node_id, &nonce, sent_unix_ms, HUB_SOCKET);
        HandshakeRequest {
            protocol_version,
            capabilities: Vec::new(),
            app_version: APP_VERSION.to_string(),
            daemon_id: machine.daemon_id.clone(),
            daemon_nonce: nonce,
            sent_unix_ms,
            daemon_signature: sign_challenge(&machine.daemon, &signed),
            hub_socket_path: HUB_SOCKET.to_string(),
        }
    }

    async fn handshake(client: &mut NodeServiceClient<Channel>, request: HandshakeRequest) -> HandshakeResponse {
        client.handshake(request).await.unwrap().into_inner()
    }

    async fn start_quiet_node(
        machine: &Machine,
    ) -> (
        tokio::task::JoinHandle<Result<(), rastai_node::error::NodeError>>,
        NodeServiceClient<Channel>,
    ) {
        let sources = machine_from_fixture("no_gpu", &machine.folder, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        start_node(test_node_options(&machine.folder, sources)).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn another_protocol_version_gets_n0001_and_the_node_version() {
        let machine = machine();
        pin(&machine);
        let (node, mut client) = start_quiet_node(&machine).await;
        let response = handshake(&mut client, daemon_request(&machine, NODE_PROTOCOL_VERSION + 1)).await;
        assert_eq!(response.diagnostic.unwrap().code, "RRS-N0001");
        assert_eq!(response.protocol_version, NODE_PROTOCOL_VERSION);
        assert_eq!(response.node_id, machine.node_id);
        assert!(response.session_token.is_empty());
        node.abort();
        let _ = fs::remove_dir_all(&machine.folder);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unknown_daemon_key_gets_n0002() {
        let machine = machine();
        let (node, mut client) = start_quiet_node(&machine).await;
        let response = handshake(&mut client, daemon_request(&machine, NODE_PROTOCOL_VERSION)).await;
        let refusal = response.diagnostic.unwrap();
        assert_eq!(refusal.code, "RRS-N0002");
        assert!(refusal.message.contains(&machine.node_id), "{}", refusal.message);
        assert!(response.session_token.is_empty());
        assert!(response.node_signature.is_empty());
        node.abort();
        let _ = fs::remove_dir_all(&machine.folder);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pinned_daemon_gets_a_signed_session_and_the_hardware() {
        let machine = machine();
        pin(&machine);
        let (node, mut client) = start_quiet_node(&machine).await;
        let request = daemon_request(&machine, NODE_PROTOCOL_VERSION);
        let nonce = request.daemon_nonce.clone();
        let response = handshake(&mut client, request.clone()).await;
        assert_eq!(response.diagnostic, None);
        assert_eq!(response.app_version, APP_VERSION);
        assert_eq!(response.hardware.unwrap().backend, "cpu");
        // The daemon checks the node's signature with the key it derives from the node id it pinned.
        let node_key = rastai_node::identity::public_key_from_id(&machine.node_id, "node_").unwrap();
        let signed = node_challenge_bytes(&machine.daemon_id, &nonce, &response.session_token);
        assert!(verify_challenge(&node_key, &signed, &response.node_signature).is_ok());
        // The same request again is a replay.
        let replayed = handshake(&mut client, request).await;
        assert_eq!(replayed.diagnostic.unwrap().code, "RRS-N0007");
        node.abort();
        let _ = fs::remove_dir_all(&machine.folder);
    }

    // The stand-in nvidia-smi prints the recorded stream; GetHardware returns the profile and those samples.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_hardware_returns_the_profile_and_live_telemetry() {
        let machine = machine();
        let sources = machine_from_fixture(
            "rtx5080_rtx3090_gfx1036",
            &machine.folder,
            NvidiaSmi::Recorded,
            TOOL_TIMEOUT,
        );
        let (node, mut client) = start_node(test_node_options(&machine.folder, sources)).await;
        let started = Instant::now();
        loop {
            let answer = client.get_hardware(GetHardwareRequest {}).await.unwrap().into_inner();
            assert_eq!(answer.hardware.unwrap().gpus.len(), 3);
            let telemetry = answer.telemetry.unwrap();
            assert!(telemetry.ram_total_bytes > 0);
            let sampled_gpus: Vec<u32> = telemetry.gpus.iter().map(|gpu| gpu.gpu_index).collect();
            if sampled_gpus == [0, 1] {
                // The recorded stream's last samples.
                assert_eq!(telemetry.gpus[0].utilization_percent, 97);
                assert_eq!(telemetry.gpus[1].utilization_percent, 4);
                break;
            }
            assert!(
                started.elapsed() < TELEMETRY_WAIT,
                "telemetry stayed at {sampled_gpus:?}"
            );
            tokio::time::sleep(TELEMETRY_POLL).await;
        }
        node.abort();
        let _ = fs::remove_dir_all(&machine.folder);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_node_on_the_same_machine_gets_n0003() {
        let machine = machine();
        let (node, _client) = start_quiet_node(&machine).await;
        // The same data folders, so the same node.sock; the stand-in machine is a copy of its own.
        let second_machine = machine.folder.join("second");
        let sources = machine_from_fixture("no_gpu", &second_machine, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let error = run_node(test_node_options(&machine.folder, sources)).await.unwrap_err();
        assert_eq!(error.code(), "RRS-N0003");
        let diagnostic = rastai_node::error::node_error_diagnostic(&error);
        assert!(
            diagnostic.message.contains(&std::process::id().to_string()),
            "{}",
            diagnostic.message
        );
        node.abort();
        let _ = fs::remove_dir_all(&machine.folder);
    }

    fn pin(machine: &Machine) {
        let path = paired_daemons_path(&test_paths(&machine.folder));
        pin_daemon_key(&path, &machine.daemon_id).unwrap();
        assert!(Path::new(&path).exists());
    }
}
