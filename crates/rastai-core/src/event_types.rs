//! The core event-type table (contracts 7.3, layout rust-control-plane.md 5.1): one `EventTypeRow` per core event
//! type. `rastai-server` serves `ListEventTypes` from it, and codegen writes the `event_types` list of
//! `schemas/names.json` from it (contracts 12.6). Plugin event types come from manifests at run time, never from
//! this file.
//!
//! Every row is a contract: its payload message is fixed field by field in contracts 4.3 by the milestone's contract
//! PR before the row lands here. M1-57 lands the M1 rows, whose payloads M1-09 fixed.

use rastai_proto::common::v1::{EventClass, Stability};

/// The app version the M1 event types arrived in.
const M1_VERSION: &str = "0.1.0";

/// One row of the core table: an in-process twin of the proto `EventTypeDescriptor`, because a `const` table can't
/// hold Strings (the same reason as `CommandRow`, contracts 2.4 rule 3). Same field names as the message; it leaves
/// out only `plugin_id`, which is empty for every core event type. `tests/wire_round_trip.rs` pins the field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTypeRow {
    /// `<area>.<what_happened>`, for example `job.state_changed`.
    pub event_type: &'static str,
    /// The generated enum: durable events are logged and indexed, ephemeral ones only reach the bus (contracts 7).
    pub event_class: EventClass,
    /// The payload message's full protobuf name, such as `rastai.events.v1.JobStateChanged`.
    pub payload_type: &'static str,
    /// Who emits it, as contracts 7.3's Emitter column writes it.
    pub emitter: &'static str,
    /// The generated enum. Every row is `Preview` until Phase 2 (owner decision N3).
    pub stability: Stability,
    pub since_version: &'static str,
}

/// One row per core event type (contracts 7.3), in the table's order.
pub const EVENT_TYPES: &[EventTypeRow] = &[
    m1_row(
        "job.state_changed",
        EventClass::Durable,
        "rastai.events.v1.JobStateChanged",
        "node",
    ),
    m1_row(
        "job.progress",
        EventClass::Ephemeral,
        "rastai.events.v1.JobProgress",
        "worker",
    ),
    m1_row(
        "job.log_line",
        EventClass::Ephemeral,
        "rastai.events.v1.JobLogLine",
        "node",
    ),
    m1_row(
        "run.state_changed",
        EventClass::Durable,
        "rastai.events.v1.RunStateChanged",
        "worker, chain driver",
    ),
    m1_row(
        "run.metrics",
        EventClass::Durable,
        "rastai.events.v1.RunMetrics",
        "worker",
    ),
    m1_row(
        "run.tags_changed",
        EventClass::Durable,
        "rastai.events.v1.RunTagsChanged",
        "daemon",
    ),
    m1_row(
        "checkpoint.saved",
        EventClass::Durable,
        "rastai.events.v1.CheckpointSaved",
        "worker",
    ),
    m1_row(
        "artifact.created",
        EventClass::Durable,
        "rastai.events.v1.ArtifactCreated",
        "worker, daemon",
    ),
    m1_row(
        "pin.changed",
        EventClass::Durable,
        "rastai.events.v1.PinChanged",
        "daemon",
    ),
    // The node's telemetry sample as it is: no message of its own (contracts 4.3).
    m1_row(
        "node.heartbeat",
        EventClass::Ephemeral,
        "rastai.common.v1.NodeTelemetry",
        "node",
    ),
    m1_row(
        "node.state_changed",
        EventClass::Durable,
        "rastai.events.v1.NodeStateChanged",
        "daemon",
    ),
    m1_row(
        "document.changed",
        EventClass::Durable,
        "rastai.events.v1.DocumentChanged",
        "daemon",
    ),
    m1_row(
        "command.finished",
        EventClass::Durable,
        "rastai.events.v1.CommandFinished",
        "daemon",
    ),
    m1_row(
        "plugin.state_changed",
        EventClass::Durable,
        "rastai.events.v1.PluginStateChanged",
        "daemon",
    ),
    m1_row(
        "diagnostic.reported",
        EventClass::Durable,
        "rastai.events.v1.DiagnosticReported",
        "any",
    ),
];

/// A row of the M1 event types: every one is `Preview` and arrived in the same version.
const fn m1_row(
    event_type: &'static str,
    event_class: EventClass,
    payload_type: &'static str,
    emitter: &'static str,
) -> EventTypeRow {
    EventTypeRow {
        event_type,
        event_class,
        payload_type,
        emitter,
        stability: Stability::Preview,
        since_version: M1_VERSION,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::Path;

    const PAYLOADS_PROTO: &str = "../../proto/rastai/events/v1/payloads.proto";
    const CONTRACTS: &str = "../../docs/architecture/contracts.md";
    const EVENT_TYPES_HEADING: &str = "### 7.3 ";
    // A field of EventPayload's oneof without a package is a message of this file's package.
    const PAYLOADS_PACKAGE: &str = "rastai.events.v1";
    const PLUGIN_EVENT_FIELD: &str = "plugin_event";
    const CLASS_WORDS: [(EventClass, &str); 2] =
        [(EventClass::Durable, "durable"), (EventClass::Ephemeral, "ephemeral")];

    fn read_repository_file(relative: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
    }

    /// `(field name, full message name)` for each field of `EventPayload`'s oneof: `job_state_changed`,
    /// `rastai.events.v1.JobStateChanged`. Comment lines are skipped.
    fn payload_oneof_fields() -> Vec<(String, String)> {
        let proto = read_repository_file(PAYLOADS_PROTO);
        let start = proto.find("oneof payload {").unwrap();
        let end = start + proto[start..].find('}').unwrap();
        let mut fields = Vec::new();
        for line in proto[start..end].lines().skip(1) {
            let declaration = line.trim();
            if declaration.is_empty() || declaration.starts_with("//") {
                continue;
            }
            let words: Vec<&str> = declaration.split_whitespace().collect();
            let message = if words[0].contains('.') {
                words[0].to_string()
            } else {
                format!("{PAYLOADS_PACKAGE}.{}", words[0])
            };
            fields.push((words[1].to_string(), message));
        }
        fields
    }

    /// The class and emitter cells of a contracts 7.3 row, by its event type.
    fn contracts_row(event_type: &str) -> (String, String) {
        let contracts = read_repository_file(CONTRACTS);
        let section_start = contracts.find(EVENT_TYPES_HEADING).unwrap();
        let row_start = format!("| `{event_type}`");
        let line = contracts[section_start..]
            .lines()
            .find(|line| line.starts_with(&format!("{row_start} ")))
            .unwrap_or_else(|| panic!("contracts 7.3 has no row for {event_type}"));
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        (cells[2].to_string(), cells[3].to_string())
    }

    fn class_word(event_class: EventClass) -> &'static str {
        CLASS_WORDS.iter().find(|(class, _)| *class == event_class).unwrap().1
    }

    #[test]
    fn every_row_is_a_well_formed_unique_preview_type() {
        let mut seen: Vec<&str> = Vec::new();
        for row in EVENT_TYPES {
            let segments: Vec<&str> = row.event_type.split('.').collect();
            assert_eq!(segments.len(), 2, "{}", row.event_type);
            assert!(
                segments.iter().all(|segment| !segment.is_empty()
                    && segment.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_')),
                "{}",
                row.event_type
            );
            assert!(!seen.contains(&row.event_type), "{} is listed twice", row.event_type);
            seen.push(row.event_type);
            assert_eq!(row.stability, Stability::Preview, "{}", row.event_type);
            assert_eq!(row.since_version, M1_VERSION, "{}", row.event_type);
        }
    }

    // A row whose payload isn't the oneof's field for it would make ListEventTypes name the wrong message.
    #[test]
    fn every_row_names_the_payload_of_its_oneof_field() {
        let fields = payload_oneof_fields();
        for row in EVENT_TYPES {
            let field_name = row.event_type.replace('.', "_");
            let field = fields.iter().find(|(name, _)| *name == field_name);
            let (_, message) = field.unwrap_or_else(|| panic!("EventPayload has no field {field_name}"));
            assert_eq!(row.payload_type, message, "{}", row.event_type);
        }
    }

    // The plugin field carries every plugin's event types, which come from manifests, not this table.
    #[test]
    fn every_oneof_field_but_the_plugin_one_has_a_row() {
        for (field_name, _) in payload_oneof_fields() {
            if field_name == PLUGIN_EVENT_FIELD {
                continue;
            }
            let found = EVENT_TYPES
                .iter()
                .any(|row| row.event_type.replace('.', "_") == field_name);
            assert!(found, "EventPayload.{field_name} has no row in EVENT_TYPES");
        }
    }

    #[test]
    fn class_and_emitter_are_the_ones_contracts_7_3_lists() {
        for row in EVENT_TYPES {
            let (class, emitter) = contracts_row(row.event_type);
            assert_eq!(class, class_word(row.event_class), "{}", row.event_type);
            assert_eq!(emitter, row.emitter, "{}", row.event_type);
        }
    }
}
