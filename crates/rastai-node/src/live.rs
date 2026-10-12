//! `WatchLive`: the node's ephemeral events on their way to the daemon (layout: rust-node-gateway-proto.md section
//! 7.5; contracts 4.4 and 7.1). Log lines, progress, and later tokens, live packets and peek steps never touch an
//! event log; they wait here, in memory, until the daemon's `WatchLive` stream takes them, and may be dropped.
//!
//! **One bounded queue per subject.** The subject is the event's job (`LiveEvent.job_id`). Each queue holds at most
//! `LIVE_QUEUE_CAPACITY` events; a new event on a full queue drops that queue's oldest one. So a slow daemon sees the
//! newest events, and one chatty job can't push another job's events out.
//!
//! **Taken in turn.** The stream takes one event from each subject that has one, in turn, so a job that writes a lot
//! can't hold back the others. Within a subject the order is kept.
//!
//! **Closed when the job ends.** Whoever feeds a job's events closes its subject after its last event (log_lines.rs
//! does after the job's final read). A closed subject takes no new events while its last ones wait, and goes away once
//! they are taken, so ended jobs leave nothing behind.
//!
//! One daemon watches a node, so one stream takes the events. A second `WatchLive` would share them with the first.

use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rastai_proto::node::v1::LiveEvent;
use tokio::sync::{Notify, mpsc};

/// The most events one subject's queue holds (contracts 15.10).
pub const LIVE_QUEUE_CAPACITY: usize = 4096;

/// The node's live queues, one per subject, and the wake-up for a stream waiting on them.
#[derive(Debug, Default)]
pub struct LiveHub {
    queues: Mutex<LiveQueues>,
    new_event: Notify,
}

#[derive(Debug, Default)]
struct LiveQueues {
    subjects: BTreeMap<String, SubjectQueue>,
    /// The subject the last event was taken from; the next take starts after it.
    last_taken: String,
}

#[derive(Debug, Default)]
struct SubjectQueue {
    events: VecDeque<LiveEvent>,
    /// How many of this subject's events a full queue has dropped, for the log.
    dropped: u64,
    closed: bool,
}

/// Puts an event on its subject's queue, dropping the queue's oldest event when it is full. An event for a subject that
/// is closed and still has events waiting is dropped.
pub fn publish_live_event(hub: &LiveHub, event: LiveEvent) {
    {
        let mut queues = lock_queues(hub);
        let queue = queues.subjects.entry(event.job_id.clone()).or_default();
        if queue.closed {
            return;
        }
        if queue.events.len() >= LIVE_QUEUE_CAPACITY {
            queue.events.pop_front();
            queue.dropped += 1;
        }
        queue.events.push_back(event);
    }
    // A stream that isn't waiting yet keeps the wake-up for its next wait, so no event is missed.
    hub.new_event.notify_one();
}

/// Closes a subject: it takes no new events, and it goes away once its queued events are taken.
pub fn close_live_subject(hub: &LiveHub, subject_id: &str) {
    let mut queues = lock_queues(hub);
    let Some(queue) = queues.subjects.get_mut(subject_id) else {
        return;
    };
    queue.closed = true;
    if queue.events.is_empty() {
        queues.subjects.remove(subject_id);
    }
}

/// The next event, taking subjects in turn, or None when every queue is empty.
pub fn take_live_event(hub: &LiveHub) -> Option<LiveEvent> {
    let mut queues = lock_queues(hub);
    let subject_id = next_subject_with_events(&queues)?;
    let queue = queues.subjects.get_mut(&subject_id)?;
    let event = queue.events.pop_front();
    if queue.closed && queue.events.is_empty() {
        queues.subjects.remove(&subject_id);
    }
    queues.last_taken = subject_id;
    event
}

/// How many events a full queue has dropped for this subject so far.
pub fn dropped_live_events(hub: &LiveHub, subject_id: &str) -> u64 {
    let queues = lock_queues(hub);
    queues.subjects.get(subject_id).map_or(0, |queue| queue.dropped)
}

/// Waits for the next event. Dropping the wait loses nothing: an event is only taken when this returns it.
async fn next_live_event(hub: &LiveHub) -> LiveEvent {
    loop {
        if let Some(event) = take_live_event(hub) {
            return event;
        }
        hub.new_event.notified().await;
    }
}

/// Sends the node's live events to one `WatchLive` stream until the daemon closes it.
pub async fn watch_live(hub: &LiveHub, sender: mpsc::Sender<LiveEvent>) {
    loop {
        let event = tokio::select! {
            event = next_live_event(hub) => event,
            _ = sender.closed() => return,
        };
        if sender.send(event).await.is_err() {
            return;
        }
    }
}

/// The first subject after `last_taken` that has an event, wrapping round to the start.
fn next_subject_with_events(queues: &LiveQueues) -> Option<String> {
    let after_last = (Bound::Excluded(queues.last_taken.clone()), Bound::Unbounded);
    let later = queues.subjects.range(after_last);
    let from_start = queues.subjects.iter();
    for (subject_id, queue) in later.chain(from_start) {
        if !queue.events.is_empty() {
            return Some(subject_id.clone());
        }
    }
    None
}

fn lock_queues(hub: &LiveHub) -> MutexGuard<'_, LiveQueues> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    hub.queues.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn event(job_id: &str, number: i64) -> LiveEvent {
        LiveEvent {
            job_id: job_id.to_string(),
            emitted_unix_ms: number,
            ..LiveEvent::default()
        }
    }

    fn take_all(hub: &LiveHub) -> Vec<(String, i64)> {
        let mut taken = Vec::new();
        while let Some(found) = take_live_event(hub) {
            taken.push((found.job_id, found.emitted_unix_ms));
        }
        taken
    }

    #[test]
    fn subjects_are_taken_in_turn_and_each_keeps_its_order() {
        let hub = LiveHub::default();
        for number in 0..3 {
            publish_live_event(&hub, event("job_a", number));
        }
        publish_live_event(&hub, event("job_b", 10));
        let taken = take_all(&hub);
        let expected = vec![
            ("job_a".to_string(), 0),
            ("job_b".to_string(), 10),
            ("job_a".to_string(), 1),
            ("job_a".to_string(), 2),
        ];
        assert_eq!(taken, expected);
    }

    #[test]
    fn a_closed_subject_takes_nothing_new_and_goes_away_once_empty() {
        let hub = LiveHub::default();
        publish_live_event(&hub, event("job_a", 1));
        close_live_subject(&hub, "job_a");
        publish_live_event(&hub, event("job_a", 2));
        assert_eq!(take_all(&hub), vec![("job_a".to_string(), 1)]);
        assert!(lock_queues(&hub).subjects.is_empty());
    }

    #[tokio::test]
    async fn a_waiting_stream_wakes_for_an_event_published_later() {
        let hub = std::sync::Arc::new(LiveHub::default());
        let waiting_hub = std::sync::Arc::clone(&hub);
        let waiter = tokio::spawn(async move { next_live_event(&waiting_hub).await });
        tokio::task::yield_now().await;
        publish_live_event(&hub, event("job_a", 7));
        let received = tokio::time::timeout(std::time::Duration::from_secs(10), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.emitted_unix_ms, 7);
    }
}
