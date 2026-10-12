//! The daemon-wide bus (contracts 7.2, flows/platform.md 6.3): every event a UI may see, logged or ephemeral, in one
//! order. It only notifies; durable state is always refetchable with commands, so losing it in a crash loses
//! nothing.
//!
//! - **Sequences.** Each event gets the next `bus_sequence`. The first after a daemon start is the start time in ms
//!   × 1,000, so sequences keep increasing across restarts without being stored, and a sequence a client kept from
//!   an earlier daemon is never mistaken for one of this daemon's.
//! - **The buffer** keeps the last `daemon.bus_buffer_events` events (10,000). A client that reconnects with an
//!   `after_bus_sequence` still in it gets the gap replayed (`events_after`); otherwise it gets `RRS-C0401` and
//!   refetches state. `after_bus_sequence` 0 is a new stream, which starts with the next event.
//! - **Live events** go out through a tokio broadcast channel (`subscribe`). A subscriber subscribes first and reads
//!   the buffer second, then skips live events it already replayed, so nothing falls between the two.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, EventRecord};
use tokio::sync::broadcast;

/// The buffer's size when `daemon.bus_buffer_events` isn't set (contracts 11.2).
pub const BUS_BUFFER_EVENTS: usize = 10_000;
/// The first sequence after a start is the start time in ms times this (contracts 7.2).
const SEQUENCES_PER_MILLISECOND: u64 = 1_000;
/// Live events one subscriber may fall behind by. A subscriber further behind is cut off with `RRS-C0401` and
/// resumes from the buffer, so a stalled client never holds the bus back.
const SUBSCRIBER_QUEUE_EVENTS: usize = 1_024;
/// Sequence 0 is never an event's: it asks for a new stream.
const NEW_STREAM: u64 = 0;
const CODE_EVENTS_MISSED: &str = "RRS-C0401";

/// The bus. Shared through `Server.bus`.
pub struct Bus {
    buffer: Mutex<BusBuffer>,
    live: broadcast::Sender<BusEvent>,
}

/// The newest events, oldest first, and the sequence the next one gets.
struct BusBuffer {
    events: VecDeque<BusEvent>,
    capacity: usize,
    next_sequence: u64,
}

/// What a subscriber asked for: only these event types and subjects. An empty list matches every one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusFilter {
    pub event_types: Vec<String>,
    pub subject_ids: Vec<String>,
}

/// An empty bus whose first event gets `start_unix_ms` × 1,000. A capacity of 0 still keeps one event, so the
/// sequence rules stay the same.
pub fn new_bus(capacity: usize, start_unix_ms: i64) -> Bus {
    let (live, _no_subscriber_yet) = broadcast::channel(SUBSCRIBER_QUEUE_EVENTS);
    let start_milliseconds = u64::try_from(start_unix_ms).unwrap_or(0);
    let buffer = BusBuffer {
        events: VecDeque::new(),
        capacity: capacity.max(1),
        next_sequence: start_milliseconds.saturating_mul(SEQUENCES_PER_MILLISECOND).max(1),
    };
    Bus {
        buffer: Mutex::new(buffer),
        live,
    }
}

/// Gives `event` the next sequence, keeps it in the buffer and sends it to every subscriber. Returns the sequence.
pub fn publish(bus: &Bus, mut event: BusEvent) -> u64 {
    // Sent while the buffer is locked, so subscribers receive events in sequence order.
    let mut buffer = lock_buffer(bus);
    let sequence = buffer.next_sequence;
    buffer.next_sequence += 1;
    event.bus_sequence = sequence;
    if buffer.events.len() == buffer.capacity {
        buffer.events.pop_front();
    }
    buffer.events.push_back(event.clone());
    // No subscriber is not an error: the buffer still holds the event for one that reconnects.
    let _ = bus.live.send(event);
    sequence
}

/// The buffered events after `after_bus_sequence` that match `filter`, oldest first. `RRS-C0401` when the gap
/// isn't all in the buffer any more, or the sequence isn't one this daemon gave out (it restarted since). 0 is a new
/// stream: nothing to replay.
pub fn events_after(bus: &Bus, after_bus_sequence: u64, filter: &BusFilter) -> Result<Vec<BusEvent>, Diagnostic> {
    if after_bus_sequence == NEW_STREAM {
        return Ok(Vec::new());
    }
    let buffer = lock_buffer(bus);
    let oldest_kept = match buffer.events.front() {
        Some(event) => event.bus_sequence,
        None => buffer.next_sequence,
    };
    // The client needs every event from after_bus_sequence + 1 on: the oldest one kept must not be later than that,
    // and the client can't have seen an event that wasn't published yet.
    let issued_here = after_bus_sequence < buffer.next_sequence;
    if !issued_here || after_bus_sequence < oldest_kept - 1 {
        let sequence = after_bus_sequence.to_string();
        return Err(diagnostic(CODE_EVENTS_MISSED, &[("sequence", &sequence)]));
    }
    let mut replay = Vec::new();
    for event in &buffer.events {
        if event.bus_sequence > after_bus_sequence && event_matches(filter, event) {
            replay.push(event.clone());
        }
    }
    Ok(replay)
}

/// Every event published from now on, in sequence order.
pub fn subscribe(bus: &Bus) -> broadcast::Receiver<BusEvent> {
    bus.live.subscribe()
}

/// True when the event's type and subject are ones the filter asks for.
pub fn event_matches(filter: &BusFilter, event: &BusEvent) -> bool {
    let type_wanted = filter.event_types.is_empty() || filter.event_types.contains(&event.event_type);
    let subject_wanted = filter.subject_ids.is_empty() || filter.subject_ids.contains(&event.subject_id);
    type_wanted && subject_wanted
}

/// A logged record as a bus event: the same type, class, time and payload (contracts 4.3: the daemon passes a logged
/// payload on as it is). `source` is `daemon`, `node:<node_id>` or `job:<job_id>`. The sequence is set by `publish`.
pub fn bus_event_from_record(record: &EventRecord, source: &str) -> BusEvent {
    BusEvent {
        bus_sequence: 0,
        event_type: record.event_type.clone(),
        event_class: record.event_class,
        subject_id: record_subject_id(record),
        source: source.to_string(),
        emitted_unix_ms: record.emitted_unix_ms,
        payload: record.payload.clone(),
    }
}

/// What a record is about (contracts 7.2: a run, job, node, conversation, document or plugin id). A run log's records
/// name their run; the daemon's own records name their subject in the payload.
fn record_subject_id(record: &EventRecord) -> String {
    if !record.run_id.is_empty() {
        return record.run_id.clone();
    }
    let payload = record.payload.as_ref().and_then(|payload| payload.payload.as_ref());
    let payload_subject = match payload {
        Some(Payload::JobStateChanged(change)) => change.job_id.clone(),
        Some(Payload::NodeStateChanged(change)) => change.node_id.clone(),
        Some(Payload::PluginStateChanged(change)) => change.plugin_id.clone(),
        Some(Payload::DocumentChanged(change)) => change.document_id.clone(),
        Some(Payload::RunTagsChanged(change)) => change.run_id.clone(),
        Some(Payload::PinChanged(change)) => change.subject_id.clone(),
        Some(Payload::DiagnosticReported(report)) => report.subject_id.clone(),
        Some(Payload::ArtifactCreated(artifact)) => artifact.artifact_id.clone(),
        Some(Payload::CommandFinished(finished)) => finished.journal_id.clone(),
        _ => String::new(),
    };
    if payload_subject.is_empty() {
        return record.job_id.clone();
    }
    payload_subject
}

/// The buffer stays whole even if a holder panicked: every change to it is one push or pop.
fn lock_buffer(bus: &Bus) -> MutexGuard<'_, BusBuffer> {
    match bus.buffer.lock() {
        Ok(buffer) => buffer,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_proto::events::v1::{EventPayload, NodeStateChanged};

    const START_UNIX_MS: i64 = 1_791_700_000_000;

    fn event(event_type: &str, subject_id: &str) -> BusEvent {
        BusEvent {
            event_type: event_type.to_string(),
            subject_id: subject_id.to_string(),
            ..BusEvent::default()
        }
    }

    fn sequences(events: &[BusEvent]) -> Vec<u64> {
        events.iter().map(|event| event.bus_sequence).collect()
    }

    #[test]
    fn the_first_sequence_is_the_start_time_times_1000() {
        let bus = new_bus(4, START_UNIX_MS);
        let first = publish(&bus, event("node.heartbeat", "node_local"));
        assert_eq!(first, 1_791_700_000_000_000);
        assert_eq!(publish(&bus, event("node.heartbeat", "node_local")), first + 1);
    }

    #[test]
    fn a_gap_inside_the_buffer_is_replayed_and_one_outside_it_is_rrs_c0401() {
        let bus = new_bus(3, START_UNIX_MS);
        let first = publish(&bus, event("a.one", "s"));
        for _ in 0..4 {
            publish(&bus, event("a.one", "s"));
        }
        // Five published, three kept: first + 2, + 3, + 4.
        let everything = BusFilter::default();
        assert_eq!(
            sequences(&events_after(&bus, first + 1, &everything).unwrap()),
            [first + 2, first + 3, first + 4]
        );
        assert_eq!(
            sequences(&events_after(&bus, first + 3, &everything).unwrap()),
            [first + 4]
        );
        assert!(events_after(&bus, first + 4, &everything).unwrap().is_empty());
        let too_old = events_after(&bus, first, &everything).unwrap_err();
        assert_eq!(too_old.code, CODE_EVENTS_MISSED);
        // A sequence this bus never gave out: the daemon restarted, or the client made it up.
        assert_eq!(
            events_after(&bus, first + 5, &everything).unwrap_err().code,
            CODE_EVENTS_MISSED
        );
        assert_eq!(events_after(&bus, 1, &everything).unwrap_err().code, CODE_EVENTS_MISSED);
        // 0 starts a new stream.
        assert!(events_after(&bus, NEW_STREAM, &everything).unwrap().is_empty());
    }

    #[test]
    fn an_empty_bus_has_no_gap_only_for_a_new_stream() {
        let bus = new_bus(3, START_UNIX_MS);
        assert!(
            events_after(&bus, NEW_STREAM, &BusFilter::default())
                .unwrap()
                .is_empty()
        );
        let earlier_daemon = 1_791_600_000_000_000;
        let refusal = events_after(&bus, earlier_daemon, &BusFilter::default()).unwrap_err();
        assert_eq!(refusal.code, CODE_EVENTS_MISSED);
    }

    #[test]
    fn filters_pick_types_and_subjects() {
        let bus = new_bus(10, START_UNIX_MS);
        let before = publish(&bus, event("run.metrics", "run_a"));
        publish(&bus, event("run.metrics", "run_b"));
        publish(&bus, event("job.state_changed", "run_a"));
        let metrics_of_a = BusFilter {
            event_types: vec!["run.metrics".to_string()],
            subject_ids: vec!["run_a".to_string()],
        };
        assert!(events_after(&bus, before, &metrics_of_a).unwrap().is_empty());
        let all_of_a = BusFilter {
            event_types: Vec::new(),
            subject_ids: vec!["run_a".to_string()],
        };
        let replay = events_after(&bus, before, &all_of_a).unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].event_type, "job.state_changed");
    }

    #[test]
    fn live_events_arrive_in_sequence_order() {
        let bus = new_bus(10, START_UNIX_MS);
        let mut receiver = subscribe(&bus);
        let first = publish(&bus, event("a.one", "s"));
        let second = publish(&bus, event("a.two", "s"));
        assert_eq!(receiver.try_recv().unwrap().bus_sequence, first);
        assert_eq!(receiver.try_recv().unwrap().bus_sequence, second);
    }

    #[test]
    fn a_record_becomes_an_event_about_its_subject() {
        let node_change = NodeStateChanged {
            node_id: "node_local".to_string(),
            ..NodeStateChanged::default()
        };
        let daemon_record = EventRecord {
            event_type: "node.state_changed".to_string(),
            emitted_unix_ms: 7,
            payload: Some(EventPayload {
                payload: Some(Payload::NodeStateChanged(node_change)),
            }),
            ..EventRecord::default()
        };
        let from_daemon = bus_event_from_record(&daemon_record, "daemon");
        assert_eq!(from_daemon.subject_id, "node_local");
        assert_eq!(from_daemon.source, "daemon");
        assert_eq!(from_daemon.emitted_unix_ms, 7);
        assert_eq!(from_daemon.payload, daemon_record.payload);
        let run_record = EventRecord {
            run_id: "run_a".to_string(),
            ..daemon_record
        };
        assert_eq!(bus_event_from_record(&run_record, "job:job_a").subject_id, "run_a");
    }
}
