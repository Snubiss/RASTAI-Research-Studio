//! Following a job or a stream through `EventService.Subscribe` until it ends (layout: rust-control-plane.md 9.3
//! step 6): its state changes, progress and log lines are printed as they arrive, and the terminal state decides
//! the exit code.
//!
//! The stream is binary Connect (`application/connect+proto`, layout 11.3): 5-byte envelopes, each a flags byte and
//! a big-endian u32 length, then a `BusEvent`; the last envelope (flag 2) is `{}` or `{"error": <envelope>}`.
//! A quiet job may send nothing for a long time, so a chunk that doesn't come within `CHUNK_TIMEOUT_MS` reopens
//! the subscription after the last event seen, which the daemon replays from its buffer.

use prost::Message;
use rastai_core::states::{is_terminal_job_state, job_state_name};
use rastai_core::wire::diagnostic_from_proto;
use rastai_proto::common::v1::JobState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, JobLogLine, JobProgress, JobStateChanged, SubscribeRequest};
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{HttpStream, next_body_chunk};
use serde_json::{Value, json};

use crate::daemon_client::{
    DaemonClient, diagnostic_from_json, open_event_stream, refusal_error, transport_diagnostic,
};
use crate::error::CliError;
use crate::output::{OutputSettings, print_line};
use crate::verbs::VerbOutcome;

/// The event types a followed job prints (contracts 7.3).
const JOB_EVENT_TYPES: &[&str] = &["job.state_changed", "job.progress", "job.log_line"];
/// How long one chunk may take before the subscription is reopened.
const CHUNK_TIMEOUT_MS: u64 = 600_000;
/// `after_bus_sequence` 0 is a new stream, which starts with the next event.
const NEW_STREAM: u64 = 0;
/// A job's own events carry `job:<job_id>` as their source (contracts 7.2).
const JOB_SOURCE_PREFIX: &str = "job:";

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
}

/// What the stream gave next.
enum StreamItem {
    /// Boxed: an event is large, and most items are events anyway.
    Event(Box<BusEvent>),
    /// The daemon ended the stream cleanly: it is stopping.
    Ended,
}

/// Subscribes to the job event types; open it before `Execute`, so none of the job's events is missed.
pub async fn open_job_events(client: &DaemonClient) -> Result<EventStream, CliError> {
    let stream = open_event_stream(client, &job_subscription(NEW_STREAM), CHUNK_TIMEOUT_MS).await?;
    Ok(EventStream {
        client: client.clone(),
        stream,
        pending: Vec::new(),
        last_bus_sequence: NEW_STREAM,
    })
}

/// Prints the job's events until it reaches a terminal state. A stream command's events end with the stream
/// itself, so for one (`ends_with_stream`) a clean end is its outcome rather than a lost daemon.
pub async fn follow_job(
    mut events: EventStream,
    job_id: &str,
    ends_with_stream: bool,
    settings: OutputSettings,
) -> Result<VerbOutcome, CliError> {
    let mut last_state = JobState::Queued;
    loop {
        let event = match next_event(&mut events).await? {
            StreamItem::Event(event) => *event,
            StreamItem::Ended if ends_with_stream => {
                return Ok(VerbOutcome::StreamEnded {
                    diagnostics: Vec::new(),
                });
            }
            StreamItem::Ended => return Err(stream_ended(&events)),
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

/// The next event, reading chunks as needed. A chunk timeout reopens the subscription after the last event seen.
async fn next_event(events: &mut EventStream) -> Result<StreamItem, CliError> {
    loop {
        if let Some(item) = take_envelope(events)? {
            return Ok(item);
        }
        match next_body_chunk(&mut events.stream).await {
            Ok(Some(chunk)) => events.pending.extend_from_slice(&chunk),
            Ok(None) => return Ok(StreamItem::Ended),
            Err(TransportError::TimedOut { .. }) => reopen(events).await?,
            Err(error) => {
                let refusal = transport_diagnostic(&error, &events.client.socket_path);
                return Err(CliError::DaemonUnavailable(vec![refusal]));
            }
        }
    }
}

async fn reopen(events: &mut EventStream) -> Result<(), CliError> {
    let request = job_subscription(events.last_bus_sequence);
    events.stream = open_event_stream(&events.client, &request, CHUNK_TIMEOUT_MS).await?;
    events.pending.clear();
    Ok(())
}

/// One whole envelope off the front of the pending bytes, or None until more arrive.
fn take_envelope(events: &mut EventStream) -> Result<Option<StreamItem>, CliError> {
    let Some((flags, message)) = split_envelope(&mut events.pending) else {
        return Ok(None);
    };
    if flags & END_OF_STREAM_FLAG != 0 {
        return end_of_stream(&message, events).map(Some);
    }
    let Ok(event) = BusEvent::decode(message.as_slice()) else {
        return Err(broken_stream(events, "an event that doesn't decode"));
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

/// The end-of-stream envelope: `{}` is a clean end, `{"error": ...}` a refusal with its diagnostic.
fn end_of_stream(message: &[u8], events: &EventStream) -> Result<StreamItem, CliError> {
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
        Some(refusal) => Err(refusal_error(refusal)),
        None => Err(broken_stream(events, &error.to_string())),
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
    broken_stream(events, STREAM_ENDED_REASON)
}

fn broken_stream(events: &EventStream, reason: &str) -> CliError {
    let error = TransportError::ConnectionFailed {
        reason: reason.to_string(),
    };
    CliError::DaemonUnavailable(vec![transport_diagnostic(&error, &events.client.socket_path)])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_proto::events::v1::EventPayload;

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
