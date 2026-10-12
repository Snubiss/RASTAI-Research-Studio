//! `WatchRunEvents` and `WatchLive` through a running node's NodeService over node.sock, as the daemon calls them
//! (layout: rust-node-gateway-proto.md 7.5; platform flows 6.1, 6.2 and 2.3).
//!
//! - The shared fixtures of `fixtures/event-logs/` stream as the reader reads them: every record of the good log; the
//!   whole records of the torn tail, then the end; the records before the corrupt frame, then the node's `RRS-S2002`
//!   report; a sequence gap's `RRS-S2003` report.
//! - A watch resumes after any `after_sequence`, also deep in a long log, where it starts from a kept position.
//! - A live writer keeps the stream open through a torn tail until the frame is whole; once the writer is gone the
//!   stream ends. "Live" is either a job in the node's table or a `job.json` whose process is alive (a job from
//!   before a node restart). The tests write the log themselves, and a stand-in process that leads its own process
//!   group, as a worker does, is the writer the `job.json` names.
//! - A node restart loses and repeats nothing: the new node streams exactly the records after the daemon's ack.
//! - A job's stdout and stderr lines arrive over `WatchLive`, and a full live queue drops its oldest event.
//!
//! Linux only, like the node's other socket tests: the liveness check reads /proc.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod on_linux {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use rastai_core::ids::{IdPrefix, new_id};
    use rastai_core::paths::{DataPaths, run_dir, run_events_log_path};
    use rastai_node::error::NodeError;
    use rastai_node::jobs::job_file::{JobFile, write_job_file};
    use rastai_node::jobs::signals::read_process_identity;
    use rastai_node::jobs::supervisor::{JobEnvironment, JobOptions};
    use rastai_node::live::{LIVE_QUEUE_CAPACITY, LiveHub, dropped_live_events, publish_live_event, take_live_event};
    use rastai_node::run_events::NODE_REPORT_SEQUENCE;
    use rastai_proto::common::v1::{EventClass, JobState};
    use rastai_proto::events::v1::event_payload::Payload;
    use rastai_proto::events::v1::{EventPayload, EventRecord, JobLogLine, LogStream, RunMetrics};
    use rastai_proto::node::v1::live_event::Payload as LivePayload;
    use rastai_proto::node::v1::node_service_client::NodeServiceClient;
    use rastai_proto::node::v1::{
        JobKind, JobSignal, JobSpec, ListJobsRequest, LiveEvent, SignalJobRequest, WatchLiveRequest,
        WatchRunEventsRequest,
    };
    use rastai_store::event_log::frame::encode_frame;
    use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
    use rastai_store::event_log::writer::{EventLogWriter, SyncPolicy, append_event, create_event_log};
    use tokio::task::JoinHandle;
    use tonic::Streaming;
    use tonic::transport::Channel;

    use crate::common::{
        NvidiaSmi, TestFolder, machine_from_fixture, new_test_folder, start_node, test_node_options, test_paths,
        write_tool,
    };

    const TEST_FOLDER_PREFIX: &str = "rnev";
    const HARDWARE_FIXTURE: &str = "no_gpu";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    /// The event-log fixtures, relative to this crate, and the run they were written for.
    const EVENT_LOG_FIXTURES: &str = "../../fixtures/event-logs";
    const FIXTURE_RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2S";
    const GOOD_LOG: &str = "good.log";
    const TORN_TAIL_LOG: &str = "torn_tail.log";
    const CORRUPT_MIDDLE_LOG: &str = "corrupt_middle.log";
    const SEQUENCE_GAP_LOG: &str = "sequence_gap.log";
    /// Long enough to cross two of the node's kept positions (one per 1,000 records).
    const LONG_LOG_RECORDS: u64 = 2500;
    /// How long a test waits for a message that should come.
    const WAIT_LIMIT: Duration = Duration::from_secs(30);
    /// How long a test watches for a message that must not come: several of the node's 50 ms polls.
    const QUIET_PERIOD: Duration = Duration::from_millis(400);
    const POLL: Duration = Duration::from_millis(20);
    const ENVIRONMENT_ID: &str = "cpu";
    /// A stand-in worker that writes a line to each log, notes it is ready, and waits to be killed.
    const TALKING_WORKER: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
echo "hello from stdout"
echo "hello from stderr" >&2
echo ready > "$run_folder/ready.txt"
exec sleep 60
"#;
    const READY_FILE: &str = "ready.txt";

    /// A node for the tests, stopped when dropped.
    struct TestNode {
        folder: TestFolder,
        paths: DataPaths,
        client: NodeServiceClient<Channel>,
        node: JoinHandle<Result<(), NodeError>>,
    }

    impl Drop for TestNode {
        fn drop(&mut self) {
            self.node.abort();
        }
    }

    /// A node in a new folder, with a `cpu` environment whose `bin/python` is `worker_script`.
    async fn start_test_node(worker_script: &str) -> TestNode {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let (node, client) = start_node_in(&folder.path, worker_script).await;
        TestNode {
            paths: test_paths(&folder.path),
            folder,
            client,
            node,
        }
    }

    /// Starts a node in `folder`, as a node restart does when the folder already holds one's data.
    async fn start_node_in(
        folder: &Path,
        worker_script: &str,
    ) -> (JoinHandle<Result<(), NodeError>>, NodeServiceClient<Channel>) {
        let environment_folder = folder.join("env");
        fs::create_dir_all(environment_folder.join("bin")).unwrap();
        write_tool(&environment_folder.join("bin/python"), worker_script);
        let sources = machine_from_fixture(HARDWARE_FIXTURE, folder, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let mut options = test_node_options(folder, sources);
        options.jobs = JobOptions {
            worker_register_timeout: WAIT_LIMIT * 2,
            stop_grace: WAIT_LIMIT,
            host_reconnect_timeout: WAIT_LIMIT * 2,
            environments: vec![JobEnvironment {
                environment_id: ENVIRONMENT_ID.to_string(),
                folder: environment_folder,
            }],
        };
        start_node(options).await
    }

    // ---- Logs and records ----

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(EVENT_LOG_FIXTURES)
            .join(name)
    }

    /// Copies a fixture in as its run's events.log and returns that path.
    fn install_fixture(node: &TestNode, name: &str) -> PathBuf {
        let log_path = run_events_log_path(&node.paths, FIXTURE_RUN_ID);
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        fs::copy(fixture_path(name), &log_path).unwrap();
        log_path
    }

    /// The records rastai-store's reader reads from a log, until anything else.
    fn records_in(path: &Path) -> Vec<EventRecord> {
        let mut reader = open_event_log(path).unwrap();
        let mut records = Vec::new();
        while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
            records.push(record);
        }
        records
    }

    fn metric_record(run_id: &str, step: u64) -> EventRecord {
        let mut values = BTreeMap::new();
        values.insert("train.loss".to_string(), 1.0 / (step as f64 + 1.0));
        EventRecord {
            run_id: run_id.to_string(),
            job_id: String::new(),
            emitted_unix_ms: 1_791_700_000_000 + step as i64,
            event_type: "run.metrics".to_string(),
            event_class: EventClass::Durable as i32,
            payload: Some(EventPayload {
                payload: Some(Payload::RunMetrics(RunMetrics { step, values })),
            }),
            ..EventRecord::default()
        }
    }

    /// A new events.log for the run, as a worker creates it.
    fn new_run_log(paths: &DataPaths, run_id: &str) -> EventLogWriter {
        fs::create_dir_all(run_dir(paths, run_id)).unwrap();
        create_event_log(&run_events_log_path(paths, run_id)).unwrap()
    }

    /// Appends metric records until the log's last sequence is `last_sequence`.
    fn append_metrics(writer: &mut EventLogWriter, run_id: &str, first_sequence: u64, last_sequence: u64) {
        for sequence in first_sequence..=last_sequence {
            let mut record = metric_record(run_id, sequence);
            append_event(writer, &mut record, SyncPolicy::Batched).unwrap();
        }
    }

    // ---- This test process as the run's writer ----

    /// The run's live writer, as a job from before a node restart is: a stand-in process leading its own group, named
    /// by the run's `job.json`. It is killed and reaped when the test ends it or drops it.
    struct LiveWriter {
        process: Child,
    }

    impl Drop for LiveWriter {
        fn drop(&mut self) {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }

    fn become_live_writer(paths: &DataPaths, run_id: &str) -> LiveWriter {
        let process = Command::new("sleep").arg("600").process_group(0).spawn().unwrap();
        let identity = read_process_identity(process.id()).unwrap();
        let job_file = JobFile {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: run_id.to_string(),
            pid: identity.pid,
            process_group_id: identity.process_group_id,
            process_start_ticks: identity.start_ticks,
            boot_id: identity.boot_id,
            ..JobFile::default()
        };
        write_job_file(&run_dir(paths, run_id), &job_file).unwrap();
        LiveWriter { process }
    }

    /// The writer has ended: its process is gone.
    fn end_live_writer(writer: LiveWriter) {
        drop(writer);
    }

    // ---- Watching ----

    async fn watch(node: &mut TestNode, run_id: &str, after_sequence: u64) -> Streaming<EventRecord> {
        let request = WatchRunEventsRequest {
            run_id: run_id.to_string(),
            after_sequence,
        };
        node.client.watch_run_events(request).await.unwrap().into_inner()
    }

    /// The next message, or None when the stream ended. Fails the test if neither comes in time.
    async fn next_record(stream: &mut Streaming<EventRecord>) -> Option<EventRecord> {
        let message = tokio::time::timeout(WAIT_LIMIT, stream.message()).await;
        message.expect("the stream neither sent nor ended").unwrap()
    }

    /// Everything the stream sends until it ends.
    async fn read_to_end(stream: &mut Streaming<EventRecord>) -> Vec<EventRecord> {
        let mut records = Vec::new();
        while let Some(record) = next_record(stream).await {
            records.push(record);
        }
        records
    }

    async fn take_records(stream: &mut Streaming<EventRecord>, count: usize) -> Vec<EventRecord> {
        let mut records = Vec::new();
        for _ in 0..count {
            records.push(next_record(stream).await.expect("the stream ended early"));
        }
        records
    }

    /// True when the stream sends nothing and doesn't end within `QUIET_PERIOD`.
    async fn stays_quiet(stream: &mut Streaming<EventRecord>) -> bool {
        tokio::time::timeout(QUIET_PERIOD, stream.message()).await.is_err()
    }

    fn sequences(records: &[EventRecord]) -> Vec<u64> {
        records.iter().map(|record| record.sequence).collect()
    }

    /// The code of the node's report, which must be a `diagnostic.reported` with sequence 0 about the run.
    fn report_code(report: &EventRecord, run_id: &str) -> String {
        assert_eq!(report.sequence, NODE_REPORT_SEQUENCE);
        assert_eq!(report.event_type, "diagnostic.reported");
        assert_eq!(report.run_id, run_id);
        let Some(EventPayload {
            payload: Some(Payload::DiagnosticReported(reported)),
        }) = &report.payload
        else {
            panic!("the report isn't a DiagnosticReported: {report:?}");
        };
        assert_eq!(reported.subject_id, run_id);
        reported.diagnostic.clone().unwrap_or_default().code
    }

    // ---- The fixtures ----

    #[tokio::test(flavor = "multi_thread")]
    async fn the_good_log_streams_every_record_then_ends() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let log_path = install_fixture(&node, GOOD_LOG);
        let mut stream = watch(&mut node, FIXTURE_RUN_ID, 0).await;
        let streamed = read_to_end(&mut stream).await;
        assert_eq!(streamed, records_in(&log_path));
        assert_eq!(sequences(&streamed), vec![1, 2, 3, 4]);
    }

    // A dead writer's torn frame will never be finished: the stream ends before it, and the file is left alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_torn_tail_of_a_gone_writer_ends_the_stream_before_it() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let log_path = install_fixture(&node, TORN_TAIL_LOG);
        let mut stream = watch(&mut node, FIXTURE_RUN_ID, 0).await;
        let streamed = read_to_end(&mut stream).await;
        assert_eq!(sequences(&streamed), vec![1, 2, 3]);
        assert_eq!(streamed, records_in(&fixture_path(GOOD_LOG))[..3].to_vec());
        assert_eq!(
            fs::read(&log_path).unwrap(),
            fs::read(fixture_path(TORN_TAIL_LOG)).unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_corrupt_middle_sends_the_records_before_it_then_rrs_s2002() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let log_path = install_fixture(&node, CORRUPT_MIDDLE_LOG);
        let mut stream = watch(&mut node, FIXTURE_RUN_ID, 0).await;
        let streamed = read_to_end(&mut stream).await;
        assert_eq!(streamed.len(), 2, "{streamed:?}");
        assert_eq!(streamed[0], records_in(&fixture_path(GOOD_LOG))[0]);
        assert_eq!(report_code(&streamed[1], FIXTURE_RUN_ID), "RRS-S2002");
        assert_eq!(
            fs::read(&log_path).unwrap(),
            fs::read(fixture_path(CORRUPT_MIDDLE_LOG)).unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sequence_gap_sends_the_records_before_it_then_rrs_s2003() {
        let mut node = start_test_node(TALKING_WORKER).await;
        install_fixture(&node, SEQUENCE_GAP_LOG);
        let mut stream = watch(&mut node, FIXTURE_RUN_ID, 0).await;
        let streamed = read_to_end(&mut stream).await;
        assert_eq!(sequences(&streamed), vec![1, 2, NODE_REPORT_SEQUENCE]);
        assert_eq!(report_code(&streamed[2], FIXTURE_RUN_ID), "RRS-S2003");
    }

    // A run id is a folder name, so anything else is refused with a report, and nothing is read.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_id_that_isnt_one_is_reported() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let mut stream = watch(&mut node, "../daemon", 0).await;
        let streamed = read_to_end(&mut stream).await;
        assert_eq!(streamed.len(), 1);
        assert_eq!(report_code(&streamed[0], "../daemon"), "RRS-C0002");
    }

    // ---- Resuming ----

    #[tokio::test(flavor = "multi_thread")]
    async fn a_watch_resumes_after_any_sequence() {
        let mut node = start_test_node(TALKING_WORKER).await;
        install_fixture(&node, GOOD_LOG);
        for after_sequence in 0..=5 {
            let mut stream = watch(&mut node, FIXTURE_RUN_ID, after_sequence).await;
            let expected: Vec<u64> = (after_sequence + 1..=4).collect();
            assert_eq!(
                sequences(&read_to_end(&mut stream).await),
                expected,
                "after {after_sequence}"
            );
        }

        // A long log, watched again and again: later watches start from the positions the earlier ones kept, and
        // must give exactly the same records as a read from the header.
        let run_id = new_id(IdPrefix::Run).unwrap();
        let mut writer = new_run_log(&node.paths, &run_id);
        append_metrics(&mut writer, &run_id, 1, LONG_LOG_RECORDS);
        let all_records = records_in(&run_events_log_path(&node.paths, &run_id));
        for after_sequence in [0, 999, 1000, 1001, 1500, 2000, 2499, 2500, 1200, 0, 3000] {
            let mut stream = watch(&mut node, &run_id, after_sequence).await;
            let streamed = read_to_end(&mut stream).await;
            let skip = (after_sequence as usize).min(all_records.len());
            assert_eq!(streamed, all_records[skip..].to_vec(), "after {after_sequence}");
        }
    }

    // ---- A live writer ----

    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_writer_is_followed_through_a_torn_frame_until_it_is_gone() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let run_id = new_id(IdPrefix::Run).unwrap();
        // The log doesn't exist yet: a live writer is waited for.
        fs::create_dir_all(run_dir(&node.paths, &run_id)).unwrap();
        let live_writer = become_live_writer(&node.paths, &run_id);
        let mut stream = watch(&mut node, &run_id, 0).await;
        assert!(stays_quiet(&mut stream).await);

        let mut writer = new_run_log(&node.paths, &run_id);
        append_metrics(&mut writer, &run_id, 1, 2);
        assert_eq!(sequences(&take_records(&mut stream, 2).await), vec![1, 2]);

        // Half a frame: the writer is mid-write, so the tail waits for the rest.
        let mut third = metric_record(&run_id, 3);
        third.sequence = 3;
        third.format_version = 1;
        let frame = encode_frame(&third).unwrap();
        let log_path = run_events_log_path(&node.paths, &run_id);
        let mut file = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        let half = frame.len() / 2;
        file.write_all(&frame[..half]).unwrap();
        assert!(stays_quiet(&mut stream).await);
        file.write_all(&frame[half..]).unwrap();
        assert_eq!(next_record(&mut stream).await.unwrap(), third);

        // The writer is gone: the stream ends after its last record.
        end_live_writer(live_writer);
        assert_eq!(read_to_end(&mut stream).await, Vec::<EventRecord>::new());
    }

    // A running job in the node's table is a live writer: its run's stream stays open until the job has ended.
    // Its stdout and stderr lines arrive over WatchLive meanwhile.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_running_job_keeps_its_run_open_and_its_lines_go_live() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let mut live = node.client.watch_live(WatchLiveRequest {}).await.unwrap().into_inner();
        let run_id = new_id(IdPrefix::Run).unwrap();
        let spec = JobSpec {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: run_id.clone(),
            kind: JobKind::Train as i32,
            environment_id: ENVIRONMENT_ID.to_string(),
            ..JobSpec::default()
        };
        let job_id = spec.job_id.clone();
        node.client.start_job(spec).await.unwrap();
        wait_for_file(&run_dir(&node.paths, &run_id).join(READY_FILE)).await;

        let mut lines = BTreeMap::new();
        while lines.len() < 2 {
            let event = next_live_event(&mut live).await;
            assert_eq!(event.job_id, job_id);
            let Some(LivePayload::JobLogLine(JobLogLine { stream, text })) = event.payload else {
                panic!("not a log line: {event:?}");
            };
            lines.insert(stream, text);
        }
        assert_eq!(lines[&(LogStream::Stdout as i32)], "hello from stdout");
        assert_eq!(lines[&(LogStream::Stderr as i32)], "hello from stderr");

        let mut writer = new_run_log(&node.paths, &run_id);
        append_metrics(&mut writer, &run_id, 1, 3);
        let mut stream = watch(&mut node, &run_id, 1).await;
        assert_eq!(sequences(&take_records(&mut stream, 2).await), vec![2, 3]);
        assert!(stays_quiet(&mut stream).await);

        let kill = SignalJobRequest {
            job_id: job_id.clone(),
            signal: JobSignal::Kill as i32,
        };
        node.client.signal_job(kill).await.unwrap();
        wait_for_end(&mut node, &job_id).await;
        assert_eq!(read_to_end(&mut stream).await, Vec::<EventRecord>::new());
    }

    // ---- A node restart ----

    // The daemon acknowledged 1,000 records, then the node was killed while the job kept writing. The new node knows
    // nothing of the old one's positions, and streams exactly the records after the ack: none lost, none repeated.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_node_restart_loses_and_repeats_nothing() {
        let mut node = start_test_node(TALKING_WORKER).await;
        let run_id = new_id(IdPrefix::Run).unwrap();
        let mut writer = new_run_log(&node.paths, &run_id);
        let live_writer = become_live_writer(&node.paths, &run_id);
        append_metrics(&mut writer, &run_id, 1, 1500);
        let mut stream = watch(&mut node, &run_id, 0).await;
        let acknowledged = take_records(&mut stream, 1000).await;
        assert_eq!(sequences(&acknowledged), (1..=1000).collect::<Vec<u64>>());

        node.node.abort();
        let _ = (&mut node.node).await;
        drop(stream);
        append_metrics(&mut writer, &run_id, 1501, 1600);
        let (restarted, client) = start_node_in(&node.folder.path, TALKING_WORKER).await;
        node.node = restarted;
        node.client = client;

        let mut resumed = watch(&mut node, &run_id, 1000).await;
        let mut after_restart = take_records(&mut resumed, 600).await;
        append_metrics(&mut writer, &run_id, 1601, 1610);
        after_restart.extend(take_records(&mut resumed, 10).await);
        end_live_writer(live_writer);
        after_restart.extend(read_to_end(&mut resumed).await);
        assert_eq!(sequences(&after_restart), (1001..=1610).collect::<Vec<u64>>());
        let all_records = records_in(&run_events_log_path(&node.paths, &run_id));
        assert_eq!(after_restart, all_records[1000..].to_vec());
    }

    // ---- Live queues ----

    #[test]
    fn a_full_live_queue_drops_its_oldest_event() {
        let hub = LiveHub::default();
        let extra = 3;
        for number in 0..LIVE_QUEUE_CAPACITY + extra {
            publish_live_event(&hub, live_event("job_busy", number as i64));
        }
        publish_live_event(&hub, live_event("job_quiet", 0));
        assert_eq!(dropped_live_events(&hub, "job_busy"), extra as u64);
        assert_eq!(dropped_live_events(&hub, "job_quiet"), 0);

        let mut busy = Vec::new();
        let mut quiet = Vec::new();
        while let Some(event) = take_live_event(&hub) {
            if event.job_id == "job_busy" {
                busy.push(event.emitted_unix_ms);
            } else {
                quiet.push(event.emitted_unix_ms);
            }
        }
        // The oldest ones went; the newest are all there, in order. The quiet job lost nothing to the busy one.
        let expected: Vec<i64> = (extra as i64..(LIVE_QUEUE_CAPACITY + extra) as i64).collect();
        assert_eq!(busy, expected);
        assert_eq!(quiet, vec![0]);
    }

    fn live_event(job_id: &str, number: i64) -> LiveEvent {
        LiveEvent {
            job_id: job_id.to_string(),
            emitted_unix_ms: number,
            ..LiveEvent::default()
        }
    }

    async fn next_live_event(stream: &mut Streaming<LiveEvent>) -> LiveEvent {
        let message = tokio::time::timeout(WAIT_LIMIT, stream.message()).await;
        message
            .expect("no live event came")
            .unwrap()
            .expect("the live stream ended")
    }

    // ---- Waiting ----

    async fn wait_for_file(path: &Path) {
        let started = Instant::now();
        while !path.exists() {
            assert!(started.elapsed() < WAIT_LIMIT, "{} didn't appear", path.display());
            tokio::time::sleep(POLL).await;
        }
    }

    async fn wait_for_end(node: &mut TestNode, job_id: &str) {
        let started = Instant::now();
        loop {
            let jobs = node
                .client
                .list_jobs(ListJobsRequest {})
                .await
                .unwrap()
                .into_inner()
                .jobs;
            let ended = jobs.iter().any(|info| {
                info.job_id == job_id
                    && matches!(
                        info.state(),
                        JobState::Succeeded | JobState::Failed | JobState::Cancelled
                    )
            });
            if ended {
                return;
            }
            assert!(started.elapsed() < WAIT_LIMIT, "job {job_id} didn't end");
            tokio::time::sleep(POLL).await;
        }
    }
}
