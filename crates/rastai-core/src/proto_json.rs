//! Wire messages written into JSON files, in the protobuf JSON mapping (contracts 4: proto field names in
//! `snake_case`, 64-bit integers as strings, enums by name, fields at their default value left out). prost has no
//! proto3 JSON mapping and §27 lists no crate for one, so this file writes it by hand, one function per message
//! (layout rust-control-plane.md 5.2).
//!
//! `job_spec_to_json` and `hardware_profile_to_json` (M1-43) are what the node writes into `job.json`, whose
//! `job_spec` and `hardware_profile` the worker decodes with protobuf's own `json_format` (python layout 7.3).
//! `job_spec_from_json` (M1-45) reads the `job_spec` back when a restarted node reattaches the job, and the daemon
//! reads it back from `run.json` when it launches a queued job (M1-47). `node_telemetry_to_json` (M1-47) is the
//! `telemetry` field of `node.status`'s result (contracts 6.5).
//! `tests/proto_json_golden.rs` checks them against the golden messages in `fixtures/proto-golden/`. The
//! `HardwareFingerprint` and `SamplingSettings` pairs join with the first code that reads them.
//!
//! The decode side reads what the node itself wrote into a file only its user can write (`job.json`, mode 0600), so
//! it is lenient the way `job.json`'s own reader is: a field that is missing or of another type reads as its default
//! (a decode that drops an unexpected `network_hosts` gives the job fewer permissions, never more). It accepts what the
//! mapping allows a writer to put: an enum by name or by number, a 64-bit integer as a string or as a number.
//!
//! Each function takes its message apart with a full pattern (no `..`), so a field added to the .proto fails to
//! compile here until it is written.

use rastai_proto::common::v1::{
    GpuInfo, GpuTelemetry, HardwareProfile, LeaseClass, NodeTelemetry, ProcessVram, ResourceRequest,
};
use rastai_proto::node::v1::{JobKind, JobSpec};
use serde_json::{Map, Value};

/// A protobuf JSON object, the decode side's view of one message.
type JsonObject = Map<String, Value>;

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

/// The `JobSpec` a `job_spec_to_json` object holds. The error says why the value isn't a message at all.
pub fn job_spec_from_json(value: &Value) -> Result<JobSpec, String> {
    let Some(object) = value.as_object() else {
        return Err("a JobSpec in JSON must be an object".to_string());
    };
    let resources = object
        .get("resources")
        .and_then(Value::as_object)
        .map(resource_request_from_json);
    Ok(JobSpec {
        job_id: read_text(object, "job_id"),
        run_id: read_text(object, "run_id"),
        kind: read_enum(object, "kind", job_kind_number),
        environment_id: read_text(object, "environment_id"),
        entry: read_text(object, "entry"),
        arguments_json: read_text(object, "arguments_json"),
        resources,
        network_hosts: read_text_list(object, "network_hosts"),
        data_policy: read_text(object, "data_policy"),
        checkpoint_policy_json: read_text(object, "checkpoint_policy_json"),
        plugin_id: read_text(object, "plugin_id"),
    })
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

/// A node's `NodeTelemetry` as protobuf JSON.
pub fn node_telemetry_to_json(telemetry: &NodeTelemetry) -> Value {
    let NodeTelemetry {
        sampled_unix_ms,
        gpus,
        ram_used_bytes,
        ram_total_bytes,
        disk_free_bytes,
    } = telemetry;
    let mut object = Map::new();
    put_i64(&mut object, "sampled_unix_ms", *sampled_unix_ms);
    if !gpus.is_empty() {
        let gpus = gpus.iter().map(gpu_telemetry_to_json).collect();
        object.insert("gpus".to_string(), Value::Array(gpus));
    }
    put_u64(&mut object, "ram_used_bytes", *ram_used_bytes);
    put_u64(&mut object, "ram_total_bytes", *ram_total_bytes);
    put_u64(&mut object, "disk_free_bytes", *disk_free_bytes);
    Value::Object(object)
}

fn gpu_telemetry_to_json(gpu: &GpuTelemetry) -> Value {
    let GpuTelemetry {
        gpu_index,
        utilization_percent,
        vram_used_bytes,
        vram_total_bytes,
        temperature_celsius,
        power_watts,
        processes,
    } = gpu;
    let mut object = Map::new();
    put_u32(&mut object, "gpu_index", *gpu_index);
    put_u32(&mut object, "utilization_percent", *utilization_percent);
    put_u64(&mut object, "vram_used_bytes", *vram_used_bytes);
    put_u64(&mut object, "vram_total_bytes", *vram_total_bytes);
    put_f32(&mut object, "temperature_celsius", *temperature_celsius);
    put_f32(&mut object, "power_watts", *power_watts);
    if !processes.is_empty() {
        let processes = processes.iter().map(process_vram_to_json).collect();
        object.insert("processes".to_string(), Value::Array(processes));
    }
    Value::Object(object)
}

fn process_vram_to_json(process: &ProcessVram) -> Value {
    let ProcessVram { pid, vram_bytes } = process;
    let mut object = Map::new();
    put_u32(&mut object, "pid", *pid);
    put_u64(&mut object, "vram_bytes", *vram_bytes);
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

fn resource_request_from_json(object: &JsonObject) -> ResourceRequest {
    let gpu_indices = read_list(object, "gpu_indices")
        .iter()
        .filter_map(u32_from_value)
        .collect();
    let vram_bytes_per_gpu = read_list(object, "vram_bytes_per_gpu")
        .iter()
        .filter_map(u64_from_value)
        .collect();
    ResourceRequest {
        gpu_indices,
        vram_bytes: read_u64(object, "vram_bytes"),
        vram_bytes_per_gpu,
        lease_class: read_enum(object, "lease_class", lease_class_number),
        cpu_count: read_u32(object, "cpu_count"),
        ram_bytes: read_u64(object, "ram_bytes"),
        sandbox_count: read_u32(object, "sandbox_count"),
        may_reclaim_evictable: object.get("may_reclaim_evictable").and_then(Value::as_bool),
    }
}

/// The enum value's name, or None for a number this build doesn't know (the mapping then writes the number).
fn job_kind_name(value: i32) -> Option<&'static str> {
    JobKind::try_from(value).ok().map(|kind| kind.as_str_name())
}

fn lease_class_name(value: i32) -> Option<&'static str> {
    LeaseClass::try_from(value).ok().map(|class| class.as_str_name())
}

fn job_kind_number(name: &str) -> Option<i32> {
    JobKind::from_str_name(name).map(|kind| kind as i32)
}

fn lease_class_number(name: &str) -> Option<i32> {
    LeaseClass::from_str_name(name).map(|class| class as i32)
}

fn read_text(object: &JsonObject, key: &str) -> String {
    object.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn read_list<'a>(object: &'a JsonObject, key: &str) -> &'a [Value] {
    match object.get(key).and_then(Value::as_array) {
        Some(items) => items,
        None => &[],
    }
}

fn read_text_list(object: &JsonObject, key: &str) -> Vec<String> {
    let texts = read_list(object, key).iter().filter_map(Value::as_str);
    texts.map(str::to_string).collect()
}

fn read_u32(object: &JsonObject, key: &str) -> u32 {
    object.get(key).and_then(u32_from_value).unwrap_or_default()
}

fn read_u64(object: &JsonObject, key: &str) -> u64 {
    object.get(key).and_then(u64_from_value).unwrap_or_default()
}

/// An enum by its name, or by its number (what the mapping writes for a number it doesn't know).
fn read_enum(object: &JsonObject, key: &str, number_of_name: fn(&str) -> Option<i32>) -> i32 {
    match object.get(key) {
        Some(Value::String(name)) => number_of_name(name).unwrap_or_default(),
        Some(Value::Number(number)) => number
            .as_i64()
            .and_then(|wide| i32::try_from(wide).ok())
            .unwrap_or_default(),
        _ => 0,
    }
}

/// A 32-bit integer is a JSON number; parsers also take it as a string.
fn u32_from_value(value: &Value) -> Option<u32> {
    let wide = u64_from_value(value)?;
    u32::try_from(wide).ok()
}

/// A 64-bit integer is a string in the mapping; parsers also take it as a number.
fn u64_from_value(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
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

fn put_i64(object: &mut Map<String, Value>, key: &str, value: i64) {
    if value != 0 {
        object.insert(key.to_string(), Value::from(value.to_string()));
    }
}

/// A float is a JSON number. The value goes through its shortest decimal text first, as protobuf-es and Python
/// write it, so 61.5 stays 61.5 and 0.1 doesn't become 0.10000000149011612.
fn put_f32(object: &mut Map<String, Value>, key: &str, value: f32) {
    if value == 0.0 {
        return;
    }
    let shortest: f64 = value.to_string().parse().unwrap_or(f64::from(value));
    if let Some(number) = serde_json::Number::from_f64(shortest) {
        object.insert(key.to_string(), Value::Number(number));
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
