//! The core event types as the formats generator needs them: the row of rastai-core's `EVENT_TYPES` (contracts
//! 7.3) with its payload's fields from the `buf build` descriptor set, so `schemas/events/` holds exactly the event
//! types the daemon serves from `ListEventTypes`. The twin of command_source.rs for commands.

use rastai_core::event_types::{EVENT_TYPES, EventTypeRow};
use rastai_proto::common::v1::EventClass;
use serde_json::Value;

use crate::command_source::stability_word;
use crate::descriptor_set::{PayloadField, payload_fields};
use crate::error::CodegenError;

/// One core event type, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDescription {
    /// `<area>.<what_happened>`, such as `job.state_changed`.
    pub event_type: String,
    /// `durable` or `ephemeral` (contracts 7.3).
    pub event_class: String,
    /// Who emits it, as contracts 7.3's Emitter column writes it.
    pub emitter: String,
    /// The payload message's full name, such as `rastai.events.v1.JobStateChanged`.
    pub payload_type: String,
    /// `preview`, `stable` or `deprecated`. Every event type is `preview` until Phase 2 (owner decision N3).
    pub stability: String,
    /// The app version that added the event type, written as `x-rastai-since-version`.
    pub since_version: String,
    /// The payload message's fields, in declaration order.
    pub fields: Vec<PayloadField>,
}

/// Every core event type, sorted by event type. Fails for a row whose payload message the descriptor set lacks.
pub fn event_descriptions(descriptor_set: &Value) -> Result<Vec<EventDescription>, CodegenError> {
    let mut events = Vec::new();
    for row in EVENT_TYPES {
        events.push(event_description(row, descriptor_set)?);
    }
    events.sort_by(|left, right| left.event_type.cmp(&right.event_type));
    Ok(events)
}

fn event_description(row: &EventTypeRow, descriptor_set: &Value) -> Result<EventDescription, CodegenError> {
    Ok(EventDescription {
        event_type: row.event_type.to_string(),
        event_class: event_class_word(row.event_class).to_string(),
        emitter: row.emitter.to_string(),
        payload_type: row.payload_type.to_string(),
        stability: stability_word(row.stability).to_string(),
        since_version: row.since_version.to_string(),
        fields: payload_fields(descriptor_set, row.payload_type)?,
    })
}

/// The class as contracts 7.3 writes it. rastai-core's tests refuse an unspecified class in the table.
fn event_class_word(event_class: EventClass) -> &'static str {
    match event_class {
        EventClass::Durable => "durable",
        EventClass::Ephemeral => "ephemeral",
        EventClass::Unspecified => "unspecified",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A descriptor set that has every M1 payload message, each with one string field, so the table's rows all
    /// resolve without running buf.
    fn descriptor_set_for_every_row() -> Value {
        let mut files = Vec::new();
        for row in EVENT_TYPES {
            let (package, message_name) = row.payload_type.rsplit_once('.').unwrap();
            files.push(json!({
                "package": package,
                "messageType": [{"name": message_name, "field": [
                    {"name": "subject_id", "number": 1, "label": "LABEL_OPTIONAL", "type": "TYPE_STRING"}
                ]}]
            }));
        }
        json!({ "file": files })
    }

    #[test]
    fn every_row_is_here_sorted_by_event_type() {
        let events = event_descriptions(&descriptor_set_for_every_row()).unwrap();
        assert_eq!(events.len(), EVENT_TYPES.len());
        let types: Vec<&str> = events.iter().map(|event| event.event_type.as_str()).collect();
        let mut sorted = types.clone();
        sorted.sort_unstable();
        assert_eq!(types, sorted);
    }

    #[test]
    fn a_row_reads_as_its_words_and_payload_fields() {
        let events = event_descriptions(&descriptor_set_for_every_row()).unwrap();
        let progress = events.iter().find(|event| event.event_type == "job.progress").unwrap();
        assert_eq!(progress.event_class, "ephemeral");
        assert_eq!(progress.emitter, "worker");
        assert_eq!(progress.payload_type, "rastai.events.v1.JobProgress");
        assert_eq!(progress.stability, "preview");
        assert_eq!(progress.since_version, "0.1.0");
        assert_eq!(progress.fields.len(), 1);
        assert_eq!(progress.fields[0].name, "subject_id");
    }

    #[test]
    fn a_payload_missing_from_the_descriptor_set_is_an_error() {
        let error = event_descriptions(&json!({ "file": [] })).unwrap_err();
        assert!(
            error.problem.contains("isn't in the `buf build` descriptor set"),
            "{error}"
        );
    }

    #[test]
    fn classes_are_the_contract_words() {
        assert_eq!(event_class_word(EventClass::Durable), "durable");
        assert_eq!(event_class_word(EventClass::Ephemeral), "ephemeral");
    }
}
