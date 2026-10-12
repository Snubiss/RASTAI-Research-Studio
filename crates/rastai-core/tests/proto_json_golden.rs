//! proto_json.rs against the golden messages (contracts 4, 12.5): the message each golden in
//! `fixtures/proto-golden/` holds, written by proto_json.rs, equals its `.json` golden, which protobuf-es and
//! Python's json_format agree on (`packages/proto/src/golden.test.js`, `python/rastai_proto/tests/test_golden.py`).
//! rastai-core doesn't depend on prost, so the messages are built here with the golden's values; rastai-proto's own
//! test checks that the `.json` and `.binpb` goldens are the same message. Every field of a golden is set, so a field
//! proto_json.rs forgets or spells differently fails here. Fields at their default are left out.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use rastai_core::proto_json::{hardware_profile_to_json, job_spec_from_json, job_spec_to_json};
use rastai_proto::common::v1::{GpuInfo, HardwareProfile, LeaseClass, ResourceRequest};
use rastai_proto::node::v1::{JobKind, JobSpec};
use serde_json::{Value, json};

const JOB_SPEC_GOLDEN: &str = "fixtures/proto-golden/node/JobSpec.json";
const HARDWARE_PROFILE_GOLDEN: &str = "fixtures/proto-golden/common/HardwareProfile.json";
const DRIVER: &str = "580.82.09";

fn golden_json(golden: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(golden);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The message of fixtures/proto-golden/node/JobSpec.json.
fn golden_job_spec() -> JobSpec {
    JobSpec {
        job_id: "job_01JA2B3C4D5E6F7G8H9J0K1M2Q".to_string(),
        run_id: "run_01JA2B3C4D5E6F7G8H9J0K1M2S".to_string(),
        kind: JobKind::Eval as i32,
        environment_id: "cuda13".to_string(),
        entry: "rastai_eval.suite_job:run_suite_job".to_string(),
        arguments_json: r#"{"suite":"evals/pinned.rrseval","model_ref":"hf:google/gemma-3-1b-it@main"}"#.to_string(),
        resources: Some(ResourceRequest {
            gpu_indices: vec![0, 1],
            vram_bytes: 30_064_771_072,
            vram_bytes_per_gpu: vec![12_884_901_888, 17_179_869_184],
            lease_class: LeaseClass::Pinned as i32,
            cpu_count: 8,
            ram_bytes: 34_359_738_368,
            sandbox_count: 4,
            may_reclaim_evictable: Some(false),
        }),
        network_hosts: vec!["db.lab.internal".to_string()],
        data_policy: "trusted-remote".to_string(),
        checkpoint_policy_json: r#"{"every_steps":500,"keep_last":3}"#.to_string(),
        plugin_id: "evaluation".to_string(),
    }
}

fn golden_gpu(index: u32, name: &str, architecture: &str, vram_bytes: u64) -> GpuInfo {
    GpuInfo {
        index,
        name: name.to_string(),
        architecture: architecture.to_string(),
        vram_bytes,
        vendor: "nvidia".to_string(),
        driver_version: DRIVER.to_string(),
    }
}

/// The message of fixtures/proto-golden/common/HardwareProfile.json.
fn golden_hardware_profile() -> HardwareProfile {
    HardwareProfile {
        cpu_model: "AMD Ryzen 9 7950X 16-Core Processor".to_string(),
        cpu_count: 32,
        ram_total_bytes: 68_719_476_736,
        disk_free_bytes: 1_099_511_627_776,
        gpus: vec![
            golden_gpu(1, "NVIDIA GeForce RTX 5080", "sm_120", 17_094_934_528),
            golden_gpu(2, "NVIDIA GeForce RTX 3090", "sm_86", 25_769_803_776),
        ],
        driver_version: DRIVER.to_string(),
        backend: "cuda13".to_string(),
    }
}

#[test]
fn the_job_spec_golden_writes_its_json_golden() {
    assert_eq!(job_spec_to_json(&golden_job_spec()), golden_json(JOB_SPEC_GOLDEN));
}

// A restarted node reads job.json's job_spec back (M1-45): the golden decodes to the golden message, and a spec
// written by job_spec_to_json reads back the same.
#[test]
fn the_job_spec_json_golden_reads_back_as_its_message() {
    assert_eq!(job_spec_from_json(&golden_json(JOB_SPEC_GOLDEN)), Ok(golden_job_spec()));
    let unset = JobSpec {
        resources: Some(ResourceRequest::default()),
        kind: 99,
        ..JobSpec::default()
    };
    assert_eq!(job_spec_from_json(&job_spec_to_json(&unset)), Ok(unset));
}

// The mapping lets a writer put a 64-bit integer as a number and an enum as its number; a field of another type
// reads as its default, and a value that isn't an object is no JobSpec.
#[test]
fn the_decode_takes_what_the_mapping_allows_and_defaults_the_rest() {
    let written = json!({
        "job_id": "job_a",
        "kind": 4,
        "resources": { "vram_bytes": 1024, "lease_class": "LEASE_CLASS_PINNED", "cpu_count": "8" },
        "network_hosts": "not a list",
        "plugin_id": 7
    });
    let spec = job_spec_from_json(&written).unwrap();
    assert_eq!(spec.job_id, "job_a");
    assert_eq!(spec.kind, 4);
    let resources = spec.resources.unwrap();
    assert_eq!(resources.vram_bytes, 1024);
    assert_eq!(resources.lease_class, LeaseClass::Pinned as i32);
    assert_eq!(resources.cpu_count, 8);
    assert!(spec.network_hosts.is_empty());
    assert_eq!(spec.plugin_id, "");
    assert!(job_spec_from_json(&json!([1, 2])).is_err());
    assert_eq!(job_spec_from_json(&json!({})), Ok(JobSpec::default()));
}

#[test]
fn the_hardware_profile_golden_writes_its_json_golden() {
    assert_eq!(
        hardware_profile_to_json(&golden_hardware_profile()),
        golden_json(HARDWARE_PROFILE_GOLDEN)
    );
}

#[test]
fn defaults_are_left_out() {
    assert_eq!(job_spec_to_json(&JobSpec::default()), json!({}));
    assert_eq!(hardware_profile_to_json(&HardwareProfile::default()), json!({}));
}

// `optional bool may_reclaim_evictable` set to true is still written, and unset is left out (unset reads as true).
#[test]
fn an_optional_field_is_written_only_when_set() {
    let set = JobSpec {
        resources: Some(ResourceRequest {
            may_reclaim_evictable: Some(true),
            ..ResourceRequest::default()
        }),
        ..JobSpec::default()
    };
    let unset = JobSpec {
        resources: Some(ResourceRequest::default()),
        ..JobSpec::default()
    };
    assert_eq!(
        job_spec_to_json(&set),
        json!({ "resources": { "may_reclaim_evictable": true } })
    );
    assert_eq!(job_spec_to_json(&unset), json!({ "resources": {} }));
}

// An enum number this build doesn't know is written as the number, as the mapping says.
#[test]
fn an_unknown_enum_number_is_written_as_the_number() {
    let spec = JobSpec {
        kind: 99,
        ..JobSpec::default()
    };
    assert_eq!(job_spec_to_json(&spec), json!({ "kind": 99 }));
}
