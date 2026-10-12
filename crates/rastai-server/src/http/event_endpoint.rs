//! `EventService.Subscribe` (contracts 4.3 and 7.2, flows/platform.md 6.3): one Connect server stream per window, in
//! `application/connect+proto` only (layout 11.3). The gate has already run and named the principal.
//!
//! - **Resume.** The stream starts with the gap after `after_bus_sequence` from the bus buffer, then live events.
//!   A gap that is no longer all in the buffer, or a sequence from before a restart, ends the stream at once with
//!   `RRS-C0401`, and the client refetches state with commands. 0 is a new stream, which starts with the next event.
//!   The receiver is subscribed before the buffer is read, and live events already replayed are skipped, so nothing
//!   falls between the two.
//! - **Who may listen.** Events show state, so a scoped token needs `read` (`RRS-C1005` otherwise; the owner has
//!   every scope). The events that carry conversation content also need `conversations.read`, as for a plugin
//!   iframe (frontend layout 10.5); without it they are left out of the stream.
//! - **Ends.** A token revoked while its stream is open ends it with `RRS-C1006` within `REVOCATION_CHECK_MS`
//!   (flows/platform.md 4.3); a subscriber too slow to keep up is ended with `RRS-C0401` and resumes from the buffer;
//!   a daemon that is stopping ends every stream cleanly, so the listeners can finish; a client that hangs up ends
//!   its stream's task at the next event or check.
//! - **`ListEventTypes`** joins this file with the event-type table it serves (`events/event_types.rs`).

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use rastai_core::principal::{Principal, PrincipalKind, principal_label};
use rastai_core::scopes::{Scope, scopes_allow};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::events::v1::{BusEvent, SubscribeRequest};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc};

use crate::events::bus::{BusFilter, event_matches, events_after, subscribe};
use crate::http::connect::{
    STREAM_CONTENT_TYPES, StreamChunk, bus_event_envelope, connect_error_response, decode_subscribe_request,
    end_of_stream_envelope, is_stream_content_type, stream_refusal_response, stream_response,
    unsupported_content_type_response,
};
use crate::server_state::Server;
use crate::shutdown::wait_until_stopping;

pub(crate) const SUBSCRIBE_PATH: &str = "/rastai.events.v1.EventService/Subscribe";
/// How often an open stream checks that its token wasn't revoked, and that its client is still there.
const REVOCATION_CHECK_MS: u64 = 1_000;
/// Envelopes queued for a client before the stream's task waits for it to read.
const STREAM_QUEUE_ENVELOPES: usize = 64;
/// The events that carry conversation content (frontend layout 10.5): only for a caller with `conversations.read`.
const CONTENT_EVENT_TYPES: &[&str] = &[
    "chat.token",
    "conversation.changed",
    "psychology.packet",
    "gym.peek_step",
];
/// How a scope refusal names this RPC, in place of a command id.
const SUBSCRIBE_NAME: &str = "EventService.Subscribe";
const CODE_NO_CREDENTIAL: &str = "RRS-C1002";
const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
const CODE_TOKEN_REVOKED: &str = "RRS-C1006";
const CODE_EVENTS_MISSED: &str = "RRS-C0401";

/// One open stream: who listens, what they asked for, and the last sequence sent.
struct Subscription {
    principal: Principal,
    filter: BusFilter,
    may_read_content: bool,
    last_sent_sequence: u64,
}

pub(crate) async fn subscribe_endpoint(
    State(server): State<Arc<Server>>,
    Extension(principal): Extension<Option<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(principal) = principal else {
        return connect_error_response(&diagnostic(CODE_NO_CREDENTIAL, &[]));
    };
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(is_stream_content_type) {
        return unsupported_content_type_response(content_type, STREAM_CONTENT_TYPES);
    }
    let request = match decode_subscribe_request(&body) {
        Ok(request) => request,
        Err(refusal) => return stream_refusal_response(&refusal),
    };
    if let Err(refusal) = check_may_subscribe(&principal) {
        return stream_refusal_response(&refusal);
    }
    let mut subscription = new_subscription(principal, &request);
    // Subscribed before the buffer is read: an event published in between is in both, and the replay wins.
    let live = subscribe(&server.bus);
    let replay = match events_after(&server.bus, request.after_bus_sequence, &subscription.filter) {
        Ok(replay) => replay,
        Err(refusal) => return stream_refusal_response(&refusal),
    };
    let (sender, envelopes) = mpsc::channel(STREAM_QUEUE_ENVELOPES);
    tokio::spawn(async move {
        let end = stream_events(&server, &mut subscription, replay, live, &sender).await;
        // The server is let go before the last envelope, so a stopping daemon can close its store once the
        // listeners are done.
        drop(server);
        let _ = sender.send(end_of_stream_envelope(end.as_ref())).await;
    });
    stream_response(envelopes)
}

/// A scoped token needs `read`; the owner has every scope.
fn check_may_subscribe(principal: &Principal) -> Result<(), Diagnostic> {
    if principal.kind == PrincipalKind::Owner || scopes_allow(&principal.scopes, &Scope::Read) {
        return Ok(());
    }
    let label = principal_label(principal);
    Err(diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[("scope", "read"), ("principal", &label), ("command_id", SUBSCRIBE_NAME)],
    ))
}

fn new_subscription(principal: Principal, request: &SubscribeRequest) -> Subscription {
    let may_read_content =
        principal.kind == PrincipalKind::Owner || scopes_allow(&principal.scopes, &Scope::ConversationsRead);
    Subscription {
        principal,
        filter: BusFilter {
            event_types: request.event_types.clone(),
            subject_ids: request.subject_ids.clone(),
        },
        may_read_content,
        last_sent_sequence: request.after_bus_sequence,
    }
}

/// Sends the replay, then live events, until the stream must end. Returns the error the end-of-stream envelope
/// carries, or `None` for a clean end.
async fn stream_events(
    server: &Server,
    subscription: &mut Subscription,
    replay: Vec<BusEvent>,
    mut live: broadcast::Receiver<BusEvent>,
    sender: &mpsc::Sender<StreamChunk>,
) -> Option<Diagnostic> {
    for event in replay {
        if !send_event(subscription, &event, sender).await {
            return None;
        }
    }
    let mut shutdown = server.shutdown.subscribe();
    let mut checks = tokio::time::interval(Duration::from_millis(REVOCATION_CHECK_MS));
    loop {
        tokio::select! {
            received = live.recv() => match received {
                Ok(event) => {
                    if !send_event(subscription, &event, sender).await {
                        return None;
                    }
                }
                Err(RecvError::Lagged(_)) => return Some(fell_behind(subscription)),
                Err(RecvError::Closed) => return None,
            },
            () = wait_until_stopping(&mut shutdown) => return None,
            _ = checks.tick() => {
                if token_was_revoked(server, &subscription.principal) {
                    return Some(diagnostic(CODE_TOKEN_REVOKED, &[]));
                }
                if sender.is_closed() {
                    return None;
                }
            }
        }
    }
}

/// Sends one event if the subscriber wants it and hasn't had it. False when the client has gone.
async fn send_event(subscription: &mut Subscription, event: &BusEvent, sender: &mpsc::Sender<StreamChunk>) -> bool {
    if !should_send(subscription, event) {
        return true;
    }
    subscription.last_sent_sequence = event.bus_sequence;
    sender.send(bus_event_envelope(event)).await.is_ok()
}

/// After the last one sent, matching the filter, and content only for a caller allowed to read it.
fn should_send(subscription: &Subscription, event: &BusEvent) -> bool {
    if event.bus_sequence <= subscription.last_sent_sequence {
        return false;
    }
    if !event_matches(&subscription.filter, event) {
        return false;
    }
    subscription.may_read_content || !CONTENT_EVENT_TYPES.contains(&event.event_type.as_str())
}

/// The bus dropped events this subscriber hadn't read: it resumes after the last one it got.
fn fell_behind(subscription: &Subscription) -> Diagnostic {
    let sequence = subscription.last_sent_sequence.to_string();
    diagnostic(CODE_EVENTS_MISSED, &[("sequence", &sequence)])
}

/// True when the stream belongs to a scoped token that `token.revoke` has revoked since it opened.
fn token_was_revoked(server: &Server, principal: &Principal) -> bool {
    if principal.kind != PrincipalKind::Token {
        return false;
    }
    let tokens = match server.auth.scoped_tokens.lock() {
        Ok(tokens) => tokens,
        Err(poisoned) => poisoned.into_inner(),
    };
    tokens
        .iter()
        .any(|token| token.token_id == principal.id && token.revoked)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::principal::owner_principal;

    fn token(scopes: Vec<Scope>) -> Principal {
        Principal {
            kind: PrincipalKind::Token,
            id: "tok_a".to_string(),
            scopes,
        }
    }

    fn event(sequence: u64, event_type: &str) -> BusEvent {
        BusEvent {
            bus_sequence: sequence,
            event_type: event_type.to_string(),
            ..BusEvent::default()
        }
    }

    #[test]
    fn a_token_needs_read_to_subscribe() {
        assert!(check_may_subscribe(&owner_principal()).is_ok());
        assert!(check_may_subscribe(&token(vec![Scope::Read])).is_ok());
        let refusal = check_may_subscribe(&token(vec![Scope::Chat])).unwrap_err();
        assert_eq!(refusal.code, CODE_SCOPE_NOT_GRANTED);
    }

    #[test]
    fn content_events_need_conversations_read_and_nothing_is_sent_twice() {
        let request = SubscribeRequest {
            after_bus_sequence: 10,
            ..SubscribeRequest::default()
        };
        let reader = new_subscription(token(vec![Scope::Read]), &request);
        assert!(should_send(&reader, &event(11, "run.metrics")));
        assert!(!should_send(&reader, &event(11, "chat.token")));
        assert!(!should_send(&reader, &event(10, "run.metrics")));
        let owner = new_subscription(owner_principal(), &request);
        assert!(should_send(&owner, &event(11, "chat.token")));
        let conversations = new_subscription(token(vec![Scope::Read, Scope::ConversationsRead]), &request);
        assert!(should_send(&conversations, &event(12, "conversation.changed")));
    }
}
