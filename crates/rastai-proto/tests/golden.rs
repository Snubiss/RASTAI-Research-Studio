//! Golden messages (layout: rust-node-gateway-proto.md section 3.5): every `fixtures/proto-golden/<package>/<Name>.binpb`
//! decodes into its prost struct and encodes back to exactly the same bytes. The Python and JavaScript tests compare
//! against the same files, so the three encoders agree byte for byte. prost drops fields it doesn't know, so a struct
//! that lags behind the .proto loses a field here and the bytes differ.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;

use prost::Message;
use rastai_proto::commands::v1::{
    CommandDescriptor, ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse,
};
use rastai_proto::common::v1::{
    BlobRef, Diagnostic, GpuInfo, GpuTelemetry, HardwareFingerprint, HardwareProfile, NodeTelemetry, ProcessVram,
    ResourceRequest, Stability,
};
use rastai_proto::events::v1::{
    ArtifactCreated, ArtifactParent, BusEvent, CheckpointSaved, CommandFinished, DiagnosticReported, DocumentChanged,
    EventPayload, EventRecord, EventTypeDescriptor, JobLogLine, JobProgress, JobStateChanged, ListEventTypesRequest,
    ListEventTypesResponse, NodeStateChanged, PinChanged, PluginEventPayload, PluginStateChanged, RunMetrics,
    RunStateChanged, RunTagsChanged, SubscribeRequest,
};
use rastai_proto::node::v1::{
    GetHardwareRequest, GetHardwareResponse, HandshakeRequest, HandshakeResponse, HeartbeatRequest, HeartbeatResponse,
    JobInfo, JobSpec, ListJobsRequest, ListJobsResponse, LiveEvent, SignalJobRequest, SignalJobResponse,
    WatchLiveRequest, WatchRunEventsRequest,
};
use rastai_proto::worker::v1::{
    ControlRequest, ControlResponse, HelloRequest, RegisterRequest, RegisterResponse, WorkerInfo,
};

const GOLDEN_ROOT: &str = "../../fixtures/proto-golden";
const BINARY_EXTENSION: &str = "binpb";
const JSON_EXTENSION: &str = "json";
// Descriptors written by a daemon that predates the stability field (layout 3.5, "Unset stability").
const WITHOUT_STABILITY_FOLDER: &str = "without_stability";
// A message with no fields encodes to no bytes at all, so these goldens are the only ones allowed to be empty.
const FIELDLESS_MESSAGES: &[&str] = &[
    "ListEventTypesRequest",
    "GetHardwareRequest",
    "HeartbeatResponse",
    "ListJobsRequest",
    "WatchLiveRequest",
    "HelloRequest",
];

/// Decodes golden bytes as one message type and encodes the message again.
type RoundTrip = fn(&[u8]) -> Vec<u8>;

/// One proto package: its golden folder and every message of it, by name, with its round trip. prost has no
/// reflection, so these lists are kept by hand; `every_golden_has_a_round_trip` fails when a golden file has no
/// line here.
struct GoldenPackage {
    folder: &'static str,
    messages: &'static [(&'static str, RoundTrip)],
}

const COMMON_MESSAGES: &[(&str, RoundTrip)] = &[
    ("BlobRef", round_trip::<BlobRef>),
    ("Diagnostic", round_trip::<Diagnostic>),
    ("GpuInfo", round_trip::<GpuInfo>),
    ("GpuTelemetry", round_trip::<GpuTelemetry>),
    ("HardwareFingerprint", round_trip::<HardwareFingerprint>),
    ("HardwareProfile", round_trip::<HardwareProfile>),
    ("NodeTelemetry", round_trip::<NodeTelemetry>),
    ("ProcessVram", round_trip::<ProcessVram>),
    ("ResourceRequest", round_trip::<ResourceRequest>),
];

const COMMANDS_MESSAGES: &[(&str, RoundTrip)] = &[
    ("CommandDescriptor", round_trip::<CommandDescriptor>),
    ("ExecuteRequest", round_trip::<ExecuteRequest>),
    ("ExecuteResponse", round_trip::<ExecuteResponse>),
    ("ListCommandsRequest", round_trip::<ListCommandsRequest>),
    ("ListCommandsResponse", round_trip::<ListCommandsResponse>),
];

const EVENTS_MESSAGES: &[(&str, RoundTrip)] = &[
    ("ArtifactCreated", round_trip::<ArtifactCreated>),
    ("ArtifactParent", round_trip::<ArtifactParent>),
    ("BusEvent", round_trip::<BusEvent>),
    ("CheckpointSaved", round_trip::<CheckpointSaved>),
    ("CommandFinished", round_trip::<CommandFinished>),
    ("DiagnosticReported", round_trip::<DiagnosticReported>),
    ("DocumentChanged", round_trip::<DocumentChanged>),
    ("EventPayload", round_trip::<EventPayload>),
    ("EventRecord", round_trip::<EventRecord>),
    ("EventTypeDescriptor", round_trip::<EventTypeDescriptor>),
    ("JobLogLine", round_trip::<JobLogLine>),
    ("JobProgress", round_trip::<JobProgress>),
    ("JobStateChanged", round_trip::<JobStateChanged>),
    ("ListEventTypesRequest", round_trip::<ListEventTypesRequest>),
    ("ListEventTypesResponse", round_trip::<ListEventTypesResponse>),
    ("NodeStateChanged", round_trip::<NodeStateChanged>),
    ("PinChanged", round_trip::<PinChanged>),
    ("PluginEventPayload", round_trip::<PluginEventPayload>),
    ("PluginStateChanged", round_trip::<PluginStateChanged>),
    ("RunMetrics", round_trip::<RunMetrics>),
    ("RunStateChanged", round_trip::<RunStateChanged>),
    ("RunTagsChanged", round_trip::<RunTagsChanged>),
    ("SubscribeRequest", round_trip::<SubscribeRequest>),
];

const NODE_MESSAGES: &[(&str, RoundTrip)] = &[
    ("GetHardwareRequest", round_trip::<GetHardwareRequest>),
    ("GetHardwareResponse", round_trip::<GetHardwareResponse>),
    ("HandshakeRequest", round_trip::<HandshakeRequest>),
    ("HandshakeResponse", round_trip::<HandshakeResponse>),
    ("HeartbeatRequest", round_trip::<HeartbeatRequest>),
    ("HeartbeatResponse", round_trip::<HeartbeatResponse>),
    ("JobInfo", round_trip::<JobInfo>),
    ("JobSpec", round_trip::<JobSpec>),
    ("ListJobsRequest", round_trip::<ListJobsRequest>),
    ("ListJobsResponse", round_trip::<ListJobsResponse>),
    ("LiveEvent", round_trip::<LiveEvent>),
    ("SignalJobRequest", round_trip::<SignalJobRequest>),
    ("SignalJobResponse", round_trip::<SignalJobResponse>),
    ("WatchLiveRequest", round_trip::<WatchLiveRequest>),
    ("WatchRunEventsRequest", round_trip::<WatchRunEventsRequest>),
];

const WORKER_MESSAGES: &[(&str, RoundTrip)] = &[
    ("ControlRequest", round_trip::<ControlRequest>),
    ("ControlResponse", round_trip::<ControlResponse>),
    ("HelloRequest", round_trip::<HelloRequest>),
    ("RegisterRequest", round_trip::<RegisterRequest>),
    ("RegisterResponse", round_trip::<RegisterResponse>),
    ("WorkerInfo", round_trip::<WorkerInfo>),
];

const GOLDEN_PACKAGES: &[GoldenPackage] = &[
    GoldenPackage {
        folder: "common",
        messages: COMMON_MESSAGES,
    },
    GoldenPackage {
        folder: "commands",
        messages: COMMANDS_MESSAGES,
    },
    GoldenPackage {
        folder: "events",
        messages: EVENTS_MESSAGES,
    },
    GoldenPackage {
        folder: "node",
        messages: NODE_MESSAGES,
    },
    GoldenPackage {
        folder: "worker",
        messages: WORKER_MESSAGES,
    },
];

fn round_trip<M: Message + Default>(bytes: &[u8]) -> Vec<u8> {
    let message = M::decode(bytes).expect("the golden bytes decode");
    message.encode_to_vec()
}

fn read_golden_bytes(folder: &str, message_name: &str) -> Vec<u8> {
    let path = format!("{GOLDEN_ROOT}/{folder}/{message_name}.{BINARY_EXTENSION}");
    fs::read(&path).unwrap_or_else(|error| panic!("can't read {path}: {error}"))
}

/// The names of the golden messages in one folder: every `<Name>.json`, which each have a `<Name>.binpb` beside it.
/// Subfolders such as `without_stability/` hold extra goldens and aren't counted.
fn golden_message_names(folder: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(format!("{GOLDEN_ROOT}/{folder}")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) == Some(JSON_EXTENSION) {
            names.insert(path.file_stem().unwrap().to_string_lossy().into_owned());
        }
    }
    names
}

#[test]
fn every_golden_round_trips_byte_identical() {
    for package in GOLDEN_PACKAGES {
        for (message_name, round_trip) in package.messages {
            let golden_bytes = read_golden_bytes(package.folder, message_name);
            assert!(
                !golden_bytes.is_empty() || FIELDLESS_MESSAGES.contains(message_name),
                "{message_name}: a golden sets every field, so it is never empty"
            );
            let encoded = round_trip(&golden_bytes);
            assert_eq!(
                encoded, golden_bytes,
                "{}/{message_name}: decode then encode changed the bytes",
                package.folder
            );
        }
    }
}

#[test]
fn every_golden_has_a_round_trip() {
    for package in GOLDEN_PACKAGES {
        let listed: BTreeSet<String> = package.messages.iter().map(|(name, _)| name.to_string()).collect();
        assert_eq!(
            golden_message_names(package.folder),
            listed,
            "{}: add the missing message to its list in golden.rs, or its golden file",
            package.folder
        );
    }
}

#[test]
fn descriptors_with_stability_set_decode_it() {
    let command = CommandDescriptor::decode(&*read_golden_bytes("commands", "CommandDescriptor")).unwrap();
    assert_eq!(command.stability(), Stability::Preview);

    let event_type = EventTypeDescriptor::decode(&*read_golden_bytes("events", "EventTypeDescriptor")).unwrap();
    assert_eq!(event_type.stability(), Stability::Preview);
}

// An older daemon sends descriptors without the field. They decode as the zero value, STABILITY_UNSPECIFIED,
// which every reader treats as preview (layout 3.2), and they still round-trip unchanged.
#[test]
fn descriptors_without_stability_decode_as_the_zero_value() {
    let command_folder = format!("commands/{WITHOUT_STABILITY_FOLDER}");
    let command_bytes = read_golden_bytes(&command_folder, "CommandDescriptor");
    let command = CommandDescriptor::decode(&*command_bytes).unwrap();
    assert_eq!(command.stability(), Stability::Unspecified);
    assert_eq!(command.encode_to_vec(), command_bytes);

    let event_folder = format!("events/{WITHOUT_STABILITY_FOLDER}");
    let event_bytes = read_golden_bytes(&event_folder, "EventTypeDescriptor");
    let event_type = EventTypeDescriptor::decode(&*event_bytes).unwrap();
    assert_eq!(event_type.stability(), Stability::Unspecified);
    assert_eq!(event_type.encode_to_vec(), event_bytes);
}
