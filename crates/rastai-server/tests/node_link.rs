//! The daemon and its embedded node (layout 8.1 and 8.6; flows/platform.md 2.1, 2.2 and 5.1; contracts 4.4 and
//! 9.5), without the training environment: the daemon starts the node and reaches it over node.sock with the
//! handshake, lists it, and takes its heartbeats only with the session token; `job.*` and `node.*` answer their
//! codes; `job.cancel` needs the scope of the command that started the job; a queued job is cancelled without a
//! signal; a job its node refuses ends its run `failed`, written by the daemon; a restart fails a running job the
//! node no longer lists (`ListJobs`); and a heartbeat's effects survive the node giving up on the call.
//!
//! The real TextCNN train job through all of this is `tests/train_jobs.rs`, which needs the cpu environment.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use rastai_core::paths::node_hub_socket_path;
use rastai_core::principal::owner_principal;
use rastai_core::scopes::Scope;
use rastai_core::time::now_unix_ms;
use rastai_node::handshake::request_with_session_token;
use rastai_proto::common::v1::JobState;
use rastai_proto::node::v1::node_hub_service_client::NodeHubServiceClient;
use rastai_proto::node::v1::{HeartbeatRequest, JobInfo};
use rastai_server::daemon::RunningDaemon;
use rastai_server::server_state::StartJobPause;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{JobRow, RunRow, get_job, get_run};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use rastai_transport::grpc_channel::connect_grpc_channel;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use common::{
    execute_request, ok_result, remove_test_folder, response_codes, run_command, start_test_daemon, stop_test_daemon,
    test_paths, token_principal,
};

const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2N";
const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2N";
const UNKNOWN_JOB_ID: &str = "job_01JB2B3C4D5E6F7G8H9J0K1M2N";
/// A node the daemon doesn't know, so the scheduler never launches a job queued for it.
const ELSEWHERE_NODE_ID: &str = "node_elsewhere";
/// The first heartbeat (at the handshake) can arrive before the daemon keeps the session; the next comes within
/// `nodes.heartbeat_seconds` (5).
const HEARTBEAT_WAIT: Duration = Duration::from_secs(20);
const WAIT_STEP: Duration = Duration::from_millis(50);
/// How long a test waits for a job's or a run's end that the daemon records on its own.
const END_WAIT: Duration = Duration::from_secs(20);
/// Shorter than any wait in the hub: a call given up after this was still being handled.
const GIVE_UP_AFTER: Duration = Duration::from_millis(300);
/// Well under the node's hub client timeout (5 s): a heartbeat must be answered within it.
const PROMPT_ANSWER: Duration = Duration::from_secs(2);

async fn run(
    running: &RunningDaemon,
    command_id: &str,
    arguments: Value,
) -> rastai_proto::commands::v1::ExecuteResponse {
    run_command(
        &running.server,
        &owner_principal(),
        execute_request(command_id, &arguments.to_string()),
    )
    .await
}

fn embedded_node_id(running: &RunningDaemon) -> String {
    running.embedded_node.node_id.clone()
}

fn job_row(running: &RunningDaemon, job_id: &str) -> Option<JobRow> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_job(&reader, job_id).unwrap()
}

fn run_row(running: &RunningDaemon, run_id: &str) -> Option<RunRow> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_run(&reader, run_id).unwrap()
}

/// Waits until `ready` holds, or fails the test after `limit`.
async fn wait_until(limit: Duration, what: &str, ready: impl Fn() -> bool) {
    let started = Instant::now();
    while !ready() {
        assert!(started.elapsed() < limit, "{what} didn't happen within {limit:?}");
        tokio::time::sleep(WAIT_STEP).await;
    }
}

/// A run and its train job, written as `submit_job` writes them, for a node of the test's choosing.
async fn insert_run_and_job(running: &RunningDaemon, node_id: &str, job_state: &str) {
    insert_run_and_job_of_kind(running, node_id, "train", job_state).await;
}

/// A run and its job of the given kind, written as `submit_job` writes them. The run keeps kind `train`: a job's kind
/// doesn't have to be its run's.
async fn insert_run_and_job_of_kind(running: &RunningDaemon, node_id: &str, kind: &str, job_state: &str) {
    let run = RunRow {
        run_id: RUN_ID.to_string(),
        kind: "train".to_string(),
        state: "created".to_string(),
        project_id: String::new(),
        node_id: node_id.to_string(),
        parent_run_id: String::new(),
        record_hash: String::new(),
        acked_sequence: 0,
        created_unix_ms: now_unix_ms(),
        finished_unix_ms: None,
        pinned: false,
        archive_state: String::new(),
    };
    let job = JobRow {
        job_id: JOB_ID.to_string(),
        run_id: RUN_ID.to_string(),
        node_id: node_id.to_string(),
        kind: kind.to_string(),
        state: job_state.to_string(),
        pid: None,
        process_group_id: None,
        exit_code: None,
        error_code: String::new(),
    };
    let write = IndexWrite::Batch(vec![IndexWrite::UpsertRun(run), IndexWrite::UpsertJob(job)]);
    write_index(&running.server.store.index, write).await.unwrap();
}

/// The embedded node's current session token, as its own heartbeats carry it.
fn session_token(running: &RunningDaemon) -> String {
    let links = running.server.nodes.links.lock().unwrap();
    links[0].session_token.clone()
}

/// A hub client on node-hub.sock, and a heartbeat from the embedded node with the session token and these jobs.
async fn hub_and_heartbeat(
    running: &RunningDaemon,
    paths: &rastai_core::paths::DataPaths,
    jobs: Vec<JobInfo>,
) -> (
    NodeHubServiceClient<tonic::transport::Channel>,
    tonic::Request<HeartbeatRequest>,
) {
    let channel = connect_grpc_channel(&node_hub_socket_path(paths)).await.unwrap();
    let beat = HeartbeatRequest {
        node_id: embedded_node_id(running),
        jobs,
        ..HeartbeatRequest::default()
    };
    let request = request_with_session_token(beat, &session_token(running)).unwrap();
    (NodeHubServiceClient::new(channel), request)
}

/// Waits until the job and its run both reach `state`.
async fn wait_for_job_and_run(running: &RunningDaemon, state: &'static str) {
    let server = running.server.clone();
    wait_until(END_WAIT, "the job's and the run's end", move || {
        let reader = open_index_reader(&server.store.index_readers).unwrap();
        let job_ended = get_job(&reader, JOB_ID).unwrap().is_some_and(|row| row.state == state);
        let run_ended = get_run(&reader, RUN_ID).unwrap().is_some_and(|row| row.state == state);
        job_ended && run_ended
    })
    .await;
}

// flows/platform.md 2.1: the daemon starts its node and reaches it over node.sock with the signed handshake; the
// node is listed, connected, and its heartbeats bring telemetry.
#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_starts_its_node_and_reaches_it_like_a_remote_one() {
    let paths = test_paths("nl-connect");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    assert!(node_id.starts_with("node_"), "{node_id}");

    let listed = ok_result(&run(&running, "node.list", json!({})).await);
    let nodes = listed["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "{listed}");
    assert_eq!(nodes[0]["node_id"], node_id.as_str());
    assert_eq!(nodes[0]["transport"], "local");
    assert_eq!(nodes[0]["state"], "connected");
    assert_eq!(nodes[0]["protocol_version"], 1);
    let status = ok_result(&run(&running, "system.status", json!({})).await);
    assert_eq!(status["nodes"][0]["node_id"], node_id.as_str());

    // The node's heartbeats carry the session token, so they reach the daemon and bring telemetry.
    let server = running.server.clone();
    let id = node_id.clone();
    wait_until(HEARTBEAT_WAIT, "a heartbeat with telemetry", move || {
        rastai_server::nodes::node_links::link_telemetry(&server, &id).is_some()
    })
    .await;
    let detail = ok_result(&run(&running, "node.status", json!({"node_id": node_id})).await);
    assert!(detail["telemetry"]["ram_total_bytes"].is_string(), "{detail}");
    assert_eq!(detail["jobs"], json!([]));
    assert!(detail["node"]["last_heartbeat_unix_ms"].as_u64().unwrap() > 0);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// contracts 4.4: the hub takes a call only with the current session's token (RRS-N0009 otherwise).
#[tokio::test(flavor = "multi_thread")]
async fn the_hub_refuses_a_heartbeat_without_the_session_token() {
    let paths = test_paths("nl-token");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    let channel = connect_grpc_channel(&node_hub_socket_path(&paths)).await.unwrap();
    let mut hub = NodeHubServiceClient::new(channel);
    let beat = HeartbeatRequest {
        node_id: node_id.clone(),
        ..HeartbeatRequest::default()
    };

    let refused = hub.heartbeat(beat.clone()).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::Unauthenticated);
    assert!(refused.message().starts_with("RRS-N0009"), "{}", refused.message());
    let forged = request_with_session_token(beat.clone(), "NOT-THE-TOKEN").unwrap();
    let refused = hub.heartbeat(forged).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::Unauthenticated);

    let token = session_token(&running);
    assert!(!token.is_empty());
    let accepted = request_with_session_token(beat, &token).unwrap();
    hub.heartbeat(accepted).await.unwrap();
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_jobs_and_nodes_are_refused_with_their_codes() {
    let paths = test_paths("nl-unknown");
    let running = start_test_daemon(&paths).await;
    let get = run(&running, "job.get", json!({"job_id": UNKNOWN_JOB_ID})).await;
    assert_eq!(response_codes(&get), ["RRS-J0005"]);
    let cancel = run(&running, "job.cancel", json!({"job_id": UNKNOWN_JOB_ID})).await;
    assert_eq!(response_codes(&cancel), ["RRS-J0005"]);
    let status = run(&running, "node.status", json!({"node_id": ELSEWHERE_NODE_ID})).await;
    assert_eq!(response_codes(&status), ["RRS-N0008"]);
    let listed = ok_result(&run(&running, "job.list", json!({})).await);
    assert_eq!(listed["jobs"], json!([]));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// contracts 6.4: job.cancel's row holds `training`, and the handler also checks the scope of the command that
// started the job. A `read` token is refused and the job is left as it was; a `training` token gets as far as the
// node, which doesn't know this job (RRS-J0004).
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_train_job_needs_the_scope_that_started_it() {
    let paths = test_paths("nl-scope");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    insert_run_and_job(&running, &node_id, "running").await;
    let cancel = execute_request("job.cancel", &json!({"job_id": JOB_ID}).to_string());

    let reader_only = run_command(&running.server, &token_principal(vec![Scope::Read]), cancel.clone()).await;
    assert_eq!(response_codes(&reader_only), ["RRS-C1005"]);
    assert_eq!(job_row(&running, JOB_ID).unwrap().state, "running");
    let trainer = run_command(&running.server, &token_principal(vec![Scope::Training]), cancel).await;
    assert_eq!(response_codes(&trainer), ["RRS-J0004"]);
    assert_eq!(job_row(&running, JOB_ID).unwrap().state, "running");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// contracts 9.1 and 9.5: a job that hasn't started is cancelled without a signal, and its run, which no record
// reached, is recorded cancelled by the daemon.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_job_is_cancelled_on_the_spot_and_so_is_its_run() {
    let paths = test_paths("nl-queued");
    let running = start_test_daemon(&paths).await;
    insert_run_and_job(&running, ELSEWHERE_NODE_ID, "queued").await;
    let cancelled = ok_result(&run(&running, "job.cancel", json!({"job_id": JOB_ID})).await);
    assert_eq!(cancelled["job"]["state"], "cancelled");
    assert_eq!(job_row(&running, JOB_ID).unwrap().state, "cancelled");
    assert_eq!(run_row(&running, RUN_ID).unwrap().state, "cancelled");
    // An ended job can't be cancelled again.
    let again = run(&running, "job.cancel", json!({"job_id": JOB_ID})).await;
    assert_eq!(response_codes(&again), ["RRS-J0004"]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// A dry run plans and starts nothing; a recipe M1 doesn't train is RRS-T0002.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_of_train_start_returns_the_plan_and_records_nothing() {
    let paths = test_paths("nl-dry");
    let running = start_test_daemon(&paths).await;
    let arguments =
        json!({"recipe": "classification", "design_path": "/d/textcnn.rrsmodel", "dataset_path": "/d/x.jsonl"});
    let mut request = execute_request("train.start", &arguments.to_string());
    request.dry_run = true;
    let planned = run_command(&running.server, &owner_principal(), request).await;
    let result = ok_result(&planned);
    assert_eq!(result["run_id"], "");
    assert_eq!(result["plan"]["method"], "full");
    assert_eq!(result["plan"]["dtype"], "fp32");
    assert!(planned.job_id.is_empty());
    let listed = ok_result(&run(&running, "job.list", json!({})).await);
    assert_eq!(listed["jobs"], json!([]));
    let wrong_recipe = json!({"recipe": "sft", "design_path": "/d/a", "dataset_path": "/d/b"});
    assert_eq!(
        response_codes(&run(&running, "train.start", wrong_recipe).await),
        ["RRS-T0002"]
    );
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// flows/platform.md 5.1 and contracts 9.5: train.start answers a job id at once; the scheduler launches the job on
// the embedded node, which can't run it here (no cpu environment in this test's data folder, RRS-N0004), so the job
// fails and the daemon writes its run's end, created → failed.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_the_node_refuses_fails_and_the_daemon_ends_its_run() {
    let paths = test_paths("nl-refused");
    let running = start_test_daemon(&paths).await;
    let arguments =
        json!({"recipe": "classification", "design_path": "/d/textcnn.rrsmodel", "dataset_path": "/d/x.jsonl"});
    let started = run(&running, "train.start", arguments).await;
    let result = ok_result(&started);
    let job_id = started.job_id.clone();
    let run_id = result["run_id"].as_str().unwrap().to_string();
    assert!(job_id.starts_with("job_"), "{job_id}");
    assert!(paths.data.join("runs").join(&run_id).join("run.json").exists());

    let server = running.server.clone();
    let id = run_id.clone();
    wait_until(Duration::from_secs(20), "the run's end", move || {
        let reader = open_index_reader(&server.store.index_readers).unwrap();
        get_run(&reader, &id).unwrap().is_some_and(|row| row.state == "failed")
    })
    .await;
    let job = job_row(&running, &job_id).unwrap();
    assert_eq!((job.state.as_str(), job.error_code.as_str()), ("failed", "RRS-N0004"));
    let got = ok_result(&run(&running, "job.get", json!({"job_id": job_id})).await);
    assert_eq!(got["job"]["state"], "failed");
    assert_eq!(got["job"]["kind"], "train");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// flows/platform.md 2.3 and contracts 9.5: a job the daemon has as running, which its node no longer knows, ended
// while no node watched. The reconciliation after the restart's connect compares ListJobs with the jobs rows: the
// job fails with RRS-J3002, and the daemon writes its run's end once the run's tail has read the (empty) log. The
// node's heartbeats name only the jobs it has, so only ListJobs can find this one.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_fails_a_running_job_the_node_no_longer_lists() {
    let paths = test_paths("nl-vanished");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    insert_run_and_job(&running, &node_id, "running").await;
    stop_test_daemon(running).await;

    let restarted = start_test_daemon(&paths).await;
    wait_for_job_and_run(&restarted, "failed").await;
    let job = job_row(&restarted, JOB_ID).unwrap();
    assert_eq!(job.error_code, "RRS-J3002");
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

// The node's hub client gives up on a call after 5 s and the server then drops the call's handler. A heartbeat that
// reported a job's end must still record it and settle the job's run, however long it waited for the job lock.
#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_the_node_gave_up_on_still_ends_the_job_and_its_run() {
    let paths = test_paths("nl-gave-up");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    insert_run_and_job(&running, &node_id, "running").await;
    let failed = JobInfo {
        job_id: JOB_ID.to_string(),
        state: JobState::Failed as i32,
        exit_code: Some(1),
        ..JobInfo::default()
    };
    let (mut hub, beat) = hub_and_heartbeat(&running, &paths, vec![failed]).await;

    // The job lock is busy, so the beat waits for it; the caller gives up meanwhile.
    let held = running.server.scheduler.job_state_lock.lock().await;
    let answered = tokio::time::timeout(GIVE_UP_AFTER, hub.heartbeat(beat)).await;
    assert!(answered.is_err(), "the beat was answered while the job lock was held");
    tokio::time::sleep(GIVE_UP_AFTER).await;
    drop(held);

    wait_for_job_and_run(&running, "failed").await;
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// A launch holds the launch lock across StartJob, which may take up to the node call timeout. A heartbeat never
// waits for it, so it is answered long before the node's hub client gives up.
#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_never_waits_for_a_launch_in_flight() {
    let paths = test_paths("nl-launching");
    let running = start_test_daemon(&paths).await;
    let (mut hub, beat) = hub_and_heartbeat(&running, &paths, Vec::new()).await;

    let launching = running.server.scheduler.launch_lock.lock().await;
    let answered = tokio::time::timeout(PROMPT_ANSWER, hub.heartbeat(beat)).await;
    assert!(answered.is_ok(), "the beat waited for the launch");
    answered.unwrap().unwrap();
    drop(launching);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// contracts 6.4: a job of a kind no command of this build starts can be cancelled only with system.admin. A token
// holding `training` passes the row's check, and the starting-scope check refuses it with RRS-C1005 naming
// system.admin, before the node is asked; the job keeps running. The owner passes the check and gets as far as the
// node, which doesn't know the job (RRS-J0004).
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_job_no_command_starts_needs_system_admin() {
    let paths = test_paths("nl-admin-kind");
    let running = start_test_daemon(&paths).await;
    let node_id = embedded_node_id(&running);
    insert_run_and_job_of_kind(&running, &node_id, "eval", "running").await;
    let cancel = execute_request("job.cancel", &json!({"job_id": JOB_ID}).to_string());

    let trainer = run_command(&running.server, &token_principal(vec![Scope::Training]), cancel.clone()).await;
    assert_eq!(response_codes(&trainer), ["RRS-C1005"]);
    let message = &trainer.diagnostics[0].message;
    assert!(message.contains("needs the scope system.admin"), "{message}");
    assert_eq!(job_row(&running, JOB_ID).unwrap().state, "running");
    let owner = run_command(&running.server, &owner_principal(), cancel).await;
    assert_eq!(response_codes(&owner), ["RRS-J0004"]);
    assert_eq!(job_row(&running, JOB_ID).unwrap().state, "running");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// contracts 9.1 and flows/platform.md 5.1: a launch holds its launch lock across StartJob and never the job lock, so
// a heartbeat from the node is applied while StartJob is in flight. The test holds the call just before the RPC
// (`Server.start_job_pause`, in `start_job_on_node`) and sends a heartbeat that reports the job running. A caller that
// held job_state_lock around the call would keep the heartbeat waiting; the bounded wait below then fails the test
// instead of hanging.
#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_is_applied_while_a_launch_is_in_start_job() {
    let paths = test_paths("nl-start-held");
    let running = start_test_daemon(&paths).await;
    let arguments =
        json!({"recipe": "classification", "design_path": "/d/textcnn.rrsmodel", "dataset_path": "/d/x.jsonl"});
    let (reached_sender, reached) = oneshot::channel();
    let (resume_sender, resume) = oneshot::channel();
    let pause = StartJobPause {
        reached: reached_sender,
        resume,
    };
    *running.server.start_job_pause.lock().unwrap() = Some(pause);
    let started = run(&running, "train.start", arguments).await;
    let job_id = started.job_id.clone();
    assert!(job_id.starts_with("job_"), "{job_id}");
    let launch_reached = tokio::time::timeout(END_WAIT, reached).await;
    assert!(
        matches!(launch_reached, Ok(Ok(()))),
        "the launch never reached StartJob"
    );

    let reports_running = JobInfo {
        job_id: job_id.clone(),
        state: JobState::Running as i32,
        ..JobInfo::default()
    };
    let (mut hub, beat) = hub_and_heartbeat(&running, &paths, vec![reports_running]).await;
    let answered = tokio::time::timeout(PROMPT_ANSWER, hub.heartbeat(beat)).await;
    // Read before the launch goes on, so the state is the heartbeat's, not StartJob's answer.
    let state_during_start_job = job_row(&running, &job_id).unwrap().state;
    // Let the launch go whatever the answer was, so a regression fails here rather than hanging the test.
    let _ = resume_sender.send(());
    assert!(answered.is_ok(), "the heartbeat waited for the launch's StartJob");
    answered.unwrap().unwrap();
    assert_eq!(state_during_start_job, "running");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
