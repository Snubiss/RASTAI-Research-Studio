//! Following a job through `EventService.Subscribe` until it ends (layout: rust-control-plane.md 9.3
//! step 6): its state changes, progress and log lines are printed as they arrive, and the terminal state decides
//! the exit code.
//!
//! The stream is binary Connect (`application/connect+proto`, layout 11.3): 5-byte envelopes, each a flags byte and
//! a big-endian u32 length, then a `BusEvent`; the last envelope (flag 2) is `{}` or `{"error": <envelope>}`.
//! A quiet job may send nothing for a long time, so a chunk that doesn't come within `CHUNK_TIMEOUT_MS` reopens
//! the subscription after the last event seen, which the daemon replays from its buffer. A stream that saw no event
//! yet can only reopen as a new one, and a gap the buffer no longer holds (`RRS-C0401`) too, and then an event sent
//! while no subscription was open is lost. So after every reopen the job's state is read again with `job.get`: a
//! job that ended meanwhile is reported at once instead of waited for.

use std::path::Path;

use prost::Message;
use rastai_core::states::{is_terminal_job_state, job_state_name, parse_job_state};
use rastai_core::wire::diagnostic_from_proto;
use rastai_formats::commands::job::{JobGetArgs, JobGetResult, JobSummary};
use rastai_proto::commands::v1::ExecuteRequest;
use rastai_proto::common::v1::JobState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, JobLogLine, JobProgress, JobStateChanged, SubscribeRequest};
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{HttpStream, next_body_chunk};
use serde_json::{Value, json};

use crate::daemon_client::{
    DaemonClient, diagnostic_from_json, execute, open_event_stream, refusal_error, transport_diagnostic,
};
use crate::error::CliError;
use crate::output::{OutputSettings, print_line};
use crate::verbs::VerbOutcome;

/// The event types a followed job prints (contracts 7.3).
const JOB_EVENT_TYPES: &[&str] = &["job.state_changed", "job.progress", "job.log_line"];
/// How long one chunk may take before the subscription is reopened.
pub const CHUNK_TIMEOUT_MS: u64 = 600_000;
/// `after_bus_sequence` 0 is a new stream, which starts with the next event.
const NEW_STREAM: u64 = 0;
/// A job's own events carry `job:<job_id>` as their source (contracts 7.2).
const JOB_SOURCE_PREFIX: &str = "job:";
/// The command that reads one job's row (contracts 6.4), asked after every reopen.
const JOB_GET_COMMAND_ID: &str = "job.get";
/// The daemon's answer to a resume from a gap its bus buffer no longer holds (contracts 7.4): the client starts a
/// new stream and reads the state again.
const CODE_EVENTS_MISSED: &str = "RRS-C0401";

const ENVELOPE_HEADER_BYTES: usize = 5;
const END_OF_STREAM_FLAG: u8 = 0b10;
const ERROR_KEY: &str = "error";
const DETAILS_KEY: &str = "details";
const DEBUG_KEY: &str = "debug";
const STREAM_ENDED_REASON: &str = "the event stream ended before the job did";
const PERCENT: f32 = 100.0;

/// An open subscription and what was read of it.
pub struct EventStream {
    client: DaemonClient,
    stream: HttpStream,
    /// Bytes received that don't make a whole envelope yet.
    pending: Vec<u8>,
    /// The last event's `bus_sequence`, where a reopened subscription resumes.
    last_bus_sequence: u64,
    /// How long one chunk may take before the subscription is reopened (`CHUNK_TIMEOUT_MS`; tests use less).
    chunk_timeout_ms: u64,
}

/// What the stream gave next.
enum StreamItem {
    /// Boxed: an event is large, and most items are events anyway.
    Event(Box<BusEvent>),
    /// The daemon ended the stream cleanly: it is stopping.
    Ended,
    /// The subscription was opened again; events sent while none was open may be lost.
    Reopened,
    /// The daemon can't replay the gap after the last event seen (`RRS-C0401`).
    EventsMissed,
}

/// Subscribes to the job event types; open it before `Execute`, so none of the job's events is missed. A chunk that
/// doesn't come within `chunk_timeout_ms` reopens it.
pub async fn open_job_events(client: &DaemonClient, chunk_timeout_ms: u64) -> Result<EventStream, CliError> {
    let stream = open_event_stream(client, &job_subscription(NEW_STREAM), chunk_timeout_ms).await?;
    Ok(EventStream {
        client: client.clone(),
        stream,
        pending: Vec::new(),
        last_bus_sequence: NEW_STREAM,
        chunk_timeout_ms,
    })
}

/// Prints the job's events until it reaches a terminal state. A stream that ends first lost its daemon.
pub async fn follow_job(
    mut events: EventStream,
    job_id: &str,
    settings: OutputSettings,
) -> Result<VerbOutcome, CliError> {
    let mut last_state = JobState::Queued;
    loop {
        let event = match next_event(&mut events).await? {
            StreamItem::Event(event) => *event,
            StreamItem::Ended => return Err(stream_ended(&events)),
            StreamItem::Reopened | StreamItem::EventsMissed => {
                if let Some(ended) = job_ended_while_reopening(&events.client, job_id, last_state, settings).await {
                    return Ok(ended);
                }
                continue;
            }
        };
        if !is_about_job(&event, job_id) {
            continue;
        }
        match event.payload.and_then(|payload| payload.payload) {
            Some(Payload::JobStateChanged(change)) => {
                print_state_change(&change, job_id, settings);
                let to_state = change.to_state();
                if is_terminal_job_state(to_state) {
                    return Ok(job_ended(job_id, &change, last_state));
                }
                last_state = to_state;
            }
            Some(Payload::JobProgress(progress)) => print_progress(&progress, settings),
            Some(Payload::JobLogLine(line)) => print_log_line(&line, settings),
            _ => {}
        }
    }
}

fn job_ended(job_id: &str, change: &JobStateChanged, last_state: JobState) -> VerbOutcome {
    // The event names the state it left; an older event without one falls back to the last state seen.
    let from_state = match change.from_state() {
        JobState::Unspecified => last_state,
        named => named,
    };
    VerbOutcome::JobEnded {
        job_id: job_id.to_string(),
        final_state: change.to_state(),
        from_state,
        diagnostics: change.diagnostic.iter().map(diagnostic_from_proto).collect(),
    }
}

/// The job's state read with `job.get`, when it already ended. Any other answer (the job still runs, or the daemon
/// can't answer `job.get` yet or refuses it) leaves the follower waiting for the job's events. The state it left
/// isn't in its row, so the last state seen stands in for it.
async fn job_ended_while_reopening(
    client: &DaemonClient,
    job_id: &str,
    last_state: JobState,
    settings: OutputSettings,
) -> Option<VerbOutcome> {
    let Ok(response) = execute(client, &job_get_request(job_id)).await else {
        return None;
    };
    if !response.ok {
        return None;
    }
    let job = job_of_result(&response.result_json)?;
    let final_state = ended_state(&job)?;
    print_job_read(&job, settings);
    Some(VerbOutcome::JobEnded {
        job_id: job_id.to_string(),
        final_state,
        from_state: last_state,
        diagnostics: Vec::new(),
    })
}

fn job_get_request(job_id: &str) -> ExecuteRequest {
    let arguments = JobGetArgs {
        job_id: job_id.to_string(),
    };
    ExecuteRequest {
        command_id: JOB_GET_COMMAND_ID.to_string(),
        arguments_json: serde_json::to_string(&arguments).unwrap_or_default(),
        // A read: nothing to replay, so no idempotency key.
        ..ExecuteRequest::default()
    }
}

/// The job row of a `job.get` result, or None when the result isn't one.
fn job_of_result(result_json: &str) -> Option<JobSummary> {
    match serde_json::from_str::<JobGetResult>(result_json) {
        Ok(result) => Some(result.job),
        Err(_) => None,
    }
}

/// The job's state when it is a terminal one.
fn ended_state(job: &JobSummary) -> Option<JobState> {
    let state = parse_job_state(&job.state)?;
    if is_terminal_job_state(state) {
        return Some(state);
    }
    None
}

/// A job that ended while no subscription was open: its row as `job.get` read it.
fn print_job_read(job: &JobSummary, settings: OutputSettings) {
    let text = format!("{} {}", job.job_id, job.state);
    print_line(settings, &text, &json!({ "job": job }));
}

/// An event of this job: its subject, its source, or a state change that names it.
fn is_about_job(event: &BusEvent, job_id: &str) -> bool {
    if event.subject_id == job_id || event.source == format!("{JOB_SOURCE_PREFIX}{job_id}") {
        return true;
    }
    match event.payload.as_ref().and_then(|payload| payload.payload.as_ref()) {
        Some(Payload::JobStateChanged(change)) => change.job_id == job_id,
        _ => false,
    }
}

fn print_state_change(change: &JobStateChanged, job_id: &str, settings: OutputSettings) {
    let to_state = job_state_name(change.to_state());
    let text = format!("{job_id} {to_state}");
    let value = json!({
        "event_type": JOB_EVENT_TYPES[0],
        "job_id": job_id,
        "from_state": job_state_name(change.from_state()),
        "to_state": to_state,
        "reason": change.reason,
    });
    print_line(settings, &text, &value);
}

fn print_progress(progress: &JobProgress, settings: OutputSettings) {
    let percent = (progress.fraction * PERCENT).round();
    let text = format!("{percent}% {}", progress.message);
    let value = json!({
        "event_type": JOB_EVENT_TYPES[1],
        "fraction": progress.fraction,
        "message": progress.message,
    });
    print_line(settings, text.trim_end(), &value);
}

fn print_log_line(line: &JobLogLine, settings: OutputSettings) {
    let value = json!({
        "event_type": JOB_EVENT_TYPES[2],
        "stream": line.stream().as_str_name(),
        "text": line.text,
    });
    print_line(settings, &line.text, &value);
}

/// The next event, reading chunks as needed. A chunk timeout reopens the subscription after the last event seen,
/// and a gap the daemon can't replay opens a new one; both say so with `Reopened`.
async fn next_event(events: &mut EventStream) -> Result<StreamItem, CliError> {
    loop {
        match take_envelope(events)? {
            Some(StreamItem::EventsMissed) => {
                events.last_bus_sequence = NEW_STREAM;
                reopen(events).await?;
                return Ok(StreamItem::Reopened);
            }
            Some(item) => return Ok(item),
            None => {}
        }
        match next_body_chunk(&mut events.stream).await {
            Ok(Some(chunk)) => events.pending.extend_from_slice(&chunk),
            Ok(None) => return Ok(StreamItem::Ended),
            Err(TransportError::TimedOut { .. }) => {
                reopen(events).await?;
                return Ok(StreamItem::Reopened);
            }
            Err(error) => {
                let refusal = transport_diagnostic(&error, &events.client.socket_path);
                return Err(CliError::DaemonUnavailable(vec![refusal]));
            }
        }
    }
}

async fn reopen(events: &mut EventStream) -> Result<(), CliError> {
    let request = job_subscription(events.last_bus_sequence);
    events.stream = open_event_stream(&events.client, &request, events.chunk_timeout_ms).await?;
    events.pending.clear();
    Ok(())
}

/// One whole envelope off the front of the pending bytes, or None until more arrive.
fn take_envelope(events: &mut EventStream) -> Result<Option<StreamItem>, CliError> {
    let Some((flags, message)) = split_envelope(&mut events.pending) else {
        return Ok(None);
    };
    if flags & END_OF_STREAM_FLAG != 0 {
        return end_of_stream(&message, &events.client.socket_path).map(Some);
    }
    let Ok(event) = BusEvent::decode(message.as_slice()) else {
        return Err(broken_stream(
            &events.client.socket_path,
            "an event that doesn't decode",
        ));
    };
    events.last_bus_sequence = event.bus_sequence;
    Ok(Some(StreamItem::Event(Box::new(event))))
}

/// The flags and message of the first envelope in `pending`, removed from it, once all its bytes are there.
pub fn split_envelope(pending: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if pending.len() < ENVELOPE_HEADER_BYTES {
        return None;
    }
    let length_bytes = [pending[1], pending[2], pending[3], pending[4]];
    let length = u32::from_be_bytes(length_bytes) as usize;
    let end = ENVELOPE_HEADER_BYTES + length;
    if pending.len() < end {
        return None;
    }
    let flags = pending[0];
    let message = pending[ENVELOPE_HEADER_BYTES..end].to_vec();
    pending.drain(..end);
    Some((flags, message))
}

/// The end-of-stream envelope: `{}` is a clean end, `{"error": ...}` a refusal with its diagnostic, except
/// `RRS-C0401`, which only says the gap is gone.
fn end_of_stream(message: &[u8], socket_path: &Path) -> Result<StreamItem, CliError> {
    let body: Value = serde_json::from_slice(message).unwrap_or_default();
    let error = &body[ERROR_KEY];
    if error.is_null() {
        return Ok(StreamItem::Ended);
    }
    let refusal = error[DETAILS_KEY]
        .as_array()
        .and_then(|details| details.first())
        .and_then(|detail| diagnostic_from_json(&detail[DEBUG_KEY]));
    match refusal {
        Some(refusal) if refusal.code == CODE_EVENTS_MISSED => Ok(StreamItem::EventsMissed),
        Some(refusal) => Err(refusal_error(refusal)),
        None => Err(broken_stream(socket_path, &error.to_string())),
    }
}

fn job_subscription(after_bus_sequence: u64) -> SubscribeRequest {
    SubscribeRequest {
        after_bus_sequence,
        event_types: JOB_EVENT_TYPES
            .iter()
            .map(|event_type| event_type.to_string())
            .collect(),
        // Not filtered by subject: a job's log events name its run as their subject, so is_about_job decides.
        subject_ids: Vec::new(),
    }
}

fn stream_ended(events: &EventStream) -> CliError {
    broken_stream(&events.client.socket_path, STREAM_ENDED_REASON)
}

fn broken_stream(socket_path: &Path, reason: &str) -> CliError {
    let error = TransportError::ConnectionFailed {
        reason: reason.to_string(),
    };
    CliError::DaemonUnavailable(vec![transport_diagnostic(&error, socket_path)])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::diagnostic;
    use rastai_proto::events::v1::EventPayload;
    use rastai_server::http::connect::end_of_stream_envelope;

    fn envelope(flags: u8, message: &[u8]) -> Vec<u8> {
        let mut bytes = vec![flags];
        bytes.extend_from_slice(&(message.len() as u32).to_be_bytes());
        bytes.extend_from_slice(message);
        bytes
    }

    #[test]
    fn envelopes_split_only_when_whole() {
        let mut pending = envelope(0, b"abc");
        pending.extend(envelope(2, b"{}"));
        let whole = pending.clone();
        let mut partial = whole[..6].to_vec();
        assert_eq!(split_envelope(&mut partial), None);
        assert_eq!(split_envelope(&mut pending), Some((0, b"abc".to_vec())));
        assert_eq!(split_envelope(&mut pending), Some((2, b"{}".to_vec())));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_gap_the_daemon_cannot_replay_is_not_a_failure() {
        let missed = diagnostic("RRS-C0401", &[("sequence", "7")]);
        let mut pending = end_of_stream_envelope(Some(&missed));
        let (_, message) = split_envelope(&mut pending).unwrap();
        let item = end_of_stream(&message, Path::new("/s")).unwrap();
        assert!(matches!(item, StreamItem::EventsMissed));

        let refused = diagnostic("RRS-C1005", &[]);
        let mut pending = end_of_stream_envelope(Some(&refused));
        let (_, message) = split_envelope(&mut pending).unwrap();
        let refusal = end_of_stream(&message, Path::new("/s")).err().unwrap();
        assert_eq!(refusal.exit_code(), 4);
    }

    #[test]
    fn only_a_terminal_job_row_ends_the_follow() {
        let row =
            |state: &str| json!({"job": {"job_id": "job_a", "node_id": "local", "kind": "train", "state": state}});
        let ended = job_of_result(&row("succeeded").to_string()).unwrap();
        assert_eq!(ended_state(&ended), Some(JobState::Succeeded));
        let running = job_of_result(&row("running").to_string()).unwrap();
        assert_eq!(ended_state(&running), None);
        assert_eq!(job_of_result("{}"), None);
    }

    #[test]
    fn events_are_matched_to_their_job() {
        let change = JobStateChanged {
            job_id: "job_a".to_string(),
            ..JobStateChanged::default()
        };
        let by_payload = BusEvent {
            subject_id: "run_a".to_string(),
            payload: Some(EventPayload {
                payload: Some(Payload::JobStateChanged(change)),
            }),
            ..BusEvent::default()
        };
        let by_source = BusEvent {
            subject_id: "run_a".to_string(),
            source: "job:job_a".to_string(),
            ..BusEvent::default()
        };
        assert!(is_about_job(&by_payload, "job_a"));
        assert!(is_about_job(&by_source, "job_a"));
        assert!(!is_about_job(&by_source, "job_b"));
    }
}
