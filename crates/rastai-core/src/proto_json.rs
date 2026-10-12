//! Wire messages written into JSON files, in the protobuf JSON mapping (contracts 4: proto field names in
//! `snake_case`, 64-bit integers as strings, enums by name, fields at their default value left out). prost has no
//! proto3 JSON mapping and §27 lists no crate for one, so this file writes it by hand, one function per message
//! (layout rust-control-plane.md 5.2).
//!
//! `job_spec_to_json` and `hardware_profile_to_json` (M1-43) are what the node writes into `job.json`, whose
//! `job_spec` and `hardware_profile` the worker decodes with protobuf's own `json_format` (python layout 7.3).
//! `tests/proto_json_golden.rs` checks both against the golden messages in `fixtures/proto-golden/`. The decode side and
//! the `HardwareFingerprint` and `SamplingSettings` pairs join with the first code that reads them.
//!
//! Each function takes its message apart with a full pattern (no `..`), so a field added to the .proto fails to
//! compile here until it is written.

use rastai_proto::common::v1::{GpuInfo, HardwareProfile, LeaseClass, ResourceRequest};
use rastai_proto::node::v1::{JobKind, JobSpec};
use serde_json::{Map, Value};

/// The `JobSpec` as protobuf JSON.
pub fn job_spec_to_json(spec: &JobSpec) -> Value {
    let JobSpec {
        job_id,
        run_id,
        kind,
        environment_id,
        entry,
        arguments_json,
        resources,
        network_hosts,
        data_policy,
        checkpoint_policy_json,
        plugin_id,
    } = spec;
    let mut object = Map::new();
    put_text(&mut object, "job_id", job_id);
    put_text(&mut object, "run_id", run_id);
    put_enum(&mut object, "kind", *kind, job_kind_name(*kind));
    put_text(&mut object, "environment_id", environment_id);
    put_text(&mut object, "entry", entry);
    put_text(&mut object, "arguments_json", arguments_json);
    if let Some(resources) = resources {
        object.insert("resources".to_string(), resource_request_to_json(resources));
    }
    put_text_list(&mut object, "network_hosts", network_hosts);
    put_text(&mut object, "data_policy", data_policy);
    put_text(&mut object, "checkpoint_policy_json", checkpoint_policy_json);
    put_text(&mut object, "plugin_id", plugin_id);
    Value::Object(object)
}

/// The node's `HardwareProfile` as protobuf JSON.
pub fn hardware_profile_to_json(profile: &HardwareProfile) -> Value {
    let HardwareProfile {
        cpu_model,
        cpu_count,
        ram_total_bytes,
        disk_free_bytes,
        gpus,
        driver_version,
        backend,
    } = profile;
    let mut object = Map::new();
    put_text(&mut object, "cpu_model", cpu_model);
    put_u32(&mut object, "cpu_count", *cpu_count);
    put_u64(&mut object, "ram_total_bytes", *ram_total_bytes);
    put_u64(&mut object, "disk_free_bytes", *disk_free_bytes);
    if !gpus.is_empty() {
        let gpus = gpus.iter().map(gpu_info_to_json).collect();
        object.insert("gpus".to_string(), Value::Array(gpus));
    }
    put_text(&mut object, "driver_version", driver_version);
    put_text(&mut object, "backend", backend);
    Value::Object(object)
}

fn resource_request_to_json(resources: &ResourceRequest) -> Value {
    let ResourceRequest {
        gpu_indices,
        vram_bytes,
        vram_bytes_per_gpu,
        lease_class,
        cpu_count,
        ram_bytes,
        sandbox_count,
        may_reclaim_evictable,
    } = resources;
    let mut object = Map::new();
    if !gpu_indices.is_empty() {
        let indices = gpu_indices.iter().map(|index| Value::from(*index)).collect();
        object.insert("gpu_indices".to_string(), Value::Array(indices));
    }
    put_u64(&mut object, "vram_bytes", *vram_bytes);
    if !vram_bytes_per_gpu.is_empty() {
        let sizes = vram_bytes_per_gpu
            .iter()
            .map(|bytes| Value::from(bytes.to_string()))
            .collect();
        object.insert("vram_bytes_per_gpu".to_string(), Value::Array(sizes));
    }
    put_enum(&mut object, "lease_class", *lease_class, lease_class_name(*lease_class));
    put_u32(&mut object, "cpu_count", *cpu_count);
    put_u64(&mut object, "ram_bytes", *ram_bytes);
    put_u32(&mut object, "sandbox_count", *sandbox_count);
    // An `optional` field is written whenever it is set, false included: that is what `optional` is for.
    if let Some(may_reclaim) = may_reclaim_evictable {
        object.insert("may_reclaim_evictable".to_string(), Value::Bool(*may_reclaim));
    }
    Value::Object(object)
}

fn gpu_info_to_json(gpu: &GpuInfo) -> Value {
    let GpuInfo {
        index,
        name,
        architecture,
        vram_bytes,
        vendor,
        driver_version,
    } = gpu;
    let mut object = Map::new();
    put_u32(&mut object, "index", *index);
    put_text(&mut object, "name", name);
    put_text(&mut object, "architecture", architecture);
    put_u64(&mut object, "vram_bytes", *vram_bytes);
    put_text(&mut object, "vendor", vendor);
    put_text(&mut object, "driver_version", driver_version);
    Value::Object(object)
}

/// The enum value's name, or None for a number this build doesn't know (the mapping then writes the number).
fn job_kind_name(value: i32) -> Option<&'static str> {
    JobKind::try_from(value).ok().map(|kind| kind.as_str_name())
}

fn lease_class_name(value: i32) -> Option<&'static str> {
    LeaseClass::try_from(value).ok().map(|class| class.as_str_name())
}

fn put_text(object: &mut Map<String, Value>, key: &str, value: &str) {
    if !value.is_empty() {
        object.insert(key.to_string(), Value::from(value));
    }
}

fn put_text_list(object: &mut Map<String, Value>, key: &str, values: &[String]) {
    if !values.is_empty() {
        let items = values.iter().map(|value| Value::from(value.as_str())).collect();
        object.insert(key.to_string(), Value::Array(items));
    }
}

fn put_u32(object: &mut Map<String, Value>, key: &str, value: u32) {
    if value != 0 {
        object.insert(key.to_string(), Value::from(value));
    }
}

/// 64-bit integers are strings in the mapping, because JavaScript numbers can't hold every one.
fn put_u64(object: &mut Map<String, Value>, key: &str, value: u64) {
    if value != 0 {
        object.insert(key.to_string(), Value::from(value.to_string()));
    }
}

fn put_enum(object: &mut Map<String, Value>, key: &str, value: i32, name: Option<&str>) {
    if value == 0 {
        return;
    }
    let written = match name {
        Some(name) => Value::from(name),
        None => Value::from(value),
    };
    object.insert(key.to_string(), written);
}
