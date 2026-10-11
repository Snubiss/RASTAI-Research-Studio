//! GPU and memory telemetry (layout: rust-node-gateway-proto.md section 7.7): one long-running nvidia-smi reader,
//! a per-process VRAM query every 5 s, rocm-smi every 5 s on an AMD node, and RAM from /proc/meminfo. The result
//! is `NodeTelemetry` in `NodeState`, which GetHardware returns and every heartbeat carries (M1-45).
//!
//! A tool that is missing or dies leaves its GPUs out of the telemetry ("unavailable"); leases then fall back to the
//! total VRAM in the hardware profile, and jobs are not affected. A dead long-running reader is started again after
//! READER_RESTART_SECONDS. The node computes on one backend and NVIDIA wins on a box with both (flows/platform.md
//! 1.2), so only that vendor's GPUs are sampled; their indexes then never collide.
//!
//! Free disk joins `NodeTelemetry` and the profile through `rastai_store::free_bytes` once M1-25 lands it; until
//! then `disk_free_bytes` is 0, which reads as "unknown".

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rastai_core::time::now_unix_ms;
use rastai_proto::common::v1::{GpuTelemetry, NodeTelemetry, ProcessVram};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::hardware::{AMD_VENDOR, GpuAddress, HardwareSources, ToolRun, memory_info_bytes, run_tool};
use crate::node::NodeState;

/// Seconds between samples, for every reader (node layout 7.7).
const TELEMETRY_INTERVAL_SECONDS: u64 = 5;
/// A long-running reader that died is started again after this long, so a crashing tool can't spin.
const READER_RESTART_SECONDS: u64 = 30;
const NVIDIA_TELEMETRY_QUERY: &[&str] = &[
    "--query-gpu=index,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
    "--format=csv,noheader,nounits",
    "-l",
    "5",
];
const NVIDIA_TELEMETRY_FIELDS: usize = 6;
const NVIDIA_PROCESS_QUERY: &[&str] = &[
    "--query-compute-apps=pid,gpu_bus_id,used_memory",
    "--format=csv,noheader,nounits",
];
const NVIDIA_PROCESS_FIELDS: usize = 3;
const ROCM_SMI_TELEMETRY_ARGUMENTS: &[&str] = &[
    "--showuse",
    "--showmemuse",
    "--showtemp",
    "--showpower",
    "--showpids",
    "--json",
];
const MEMORY_INFO_FILE: &str = "meminfo";
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Starts the telemetry loops, one tokio task each, so a reader sees every background task here.
pub(crate) fn start_telemetry(state: &Arc<NodeState>, sources: &HardwareSources, nvidia_gpus: Vec<GpuAddress>) {
    tokio::spawn(memory_loop(Arc::clone(state), sources.proc_root.clone()));
    if !nvidia_gpus.is_empty() {
        tokio::spawn(nvidia_reader_loop(Arc::clone(state), sources.nvidia_smi.clone()));
        tokio::spawn(nvidia_process_loop(
            Arc::clone(state),
            sources.nvidia_smi.clone(),
            nvidia_gpus,
            sources.tool_timeout,
        ));
    } else if has_gpus_of_vendor(state, AMD_VENDOR) {
        tokio::spawn(rocm_smi_loop(
            Arc::clone(state),
            sources.rocm_smi.clone(),
            sources.tool_timeout,
        ));
    }
}

/// One line of nvidia-smi's telemetry query. Fields a GPU doesn't report (`[N/A]`, `[Not Supported]`) read as 0;
/// the index and memory must parse, else the line is skipped.
fn parse_nvidia_telemetry_line(line: &str) -> Option<GpuTelemetry> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() != NVIDIA_TELEMETRY_FIELDS {
        return None;
    }
    let memory_used_mib: u64 = fields[2].parse().ok()?;
    let memory_total_mib: u64 = fields[3].parse().ok()?;
    Some(GpuTelemetry {
        gpu_index: fields[0].parse().ok()?,
        utilization_percent: fields[1].parse().unwrap_or(0),
        vram_used_bytes: memory_used_mib * BYTES_PER_MIB,
        vram_total_bytes: memory_total_mib * BYTES_PER_MIB,
        temperature_celsius: fields[4].parse().unwrap_or(0.0),
        power_watts: fields[5].parse().unwrap_or(0.0),
        processes: Vec::new(),
    })
}

/// nvidia-smi's per-process query, grouped by GPU index through the bus ids found at detection. Rows of a bus id
/// detection didn't see, and malformed rows, are skipped. A process on two GPUs is listed on each.
fn parse_nvidia_processes(text: &str, gpus: &[GpuAddress]) -> Vec<(u32, ProcessVram)> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.len() != NVIDIA_PROCESS_FIELDS {
                return None;
            }
            let gpu = gpus.iter().find(|gpu| gpu.bus_id.eq_ignore_ascii_case(fields[1]))?;
            let used_mib: u64 = fields[2].parse().ok()?;
            let process = ProcessVram {
                pid: fields[0].parse().ok()?,
                vram_bytes: used_mib * BYTES_PER_MIB,
            };
            Some((gpu.index, process))
        })
        .collect()
}

/// rocm-smi's telemetry JSON, one object per `cardN`. Keys differ between ROCm releases, so each value is found by
/// words in its key, without case. rocm-smi reports memory in percent of VRAM, so the bytes come from the
/// profile's totals (`vram_totals`: index and bytes). A card whose use can't be read is skipped. Per-process rows
/// (`--showpids`) are asked for but not read yet: their layout needs a recording from a ROCm box (M10's AMD runner).
fn parse_rocm_smi_telemetry(text: &str, vram_totals: &[(u32, u64)]) -> Vec<GpuTelemetry> {
    let Ok(Value::Object(cards)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    cards
        .iter()
        .filter_map(|(card_name, fields)| {
            let gpu_index: u32 = card_name.strip_prefix("card")?.parse().ok()?;
            let fields = fields.as_object()?;
            let number = |words: &[&str]| {
                fields
                    .iter()
                    .find(|(key, _)| words.iter().all(|word| key.to_ascii_lowercase().contains(word)))
                    .and_then(|(_, value)| value.as_str()?.trim().parse::<f64>().ok())
            };
            let vram_total_bytes = vram_totals
                .iter()
                .find(|(index, _)| *index == gpu_index)
                .map(|(_, total)| *total)
                .unwrap_or(0);
            let vram_percent = number(&["vram%"]).unwrap_or(0.0);
            Some(GpuTelemetry {
                gpu_index,
                utilization_percent: number(&["gpu use"])? as u32,
                vram_used_bytes: (vram_total_bytes as f64 * vram_percent / 100.0) as u64,
                vram_total_bytes,
                temperature_celsius: number(&["temperature", "edge"]).unwrap_or(0.0) as f32,
                power_watts: number(&["power", "(w)"]).unwrap_or(0.0) as f32,
                processes: Vec::new(),
            })
        })
        .collect()
}

/// Stores one GPU's sample, keeping the per-process rows the other reader wrote.
fn store_gpu_sample(state: &NodeState, mut sample: GpuTelemetry) {
    let mut telemetry = lock_telemetry(state);
    telemetry.sampled_unix_ms = now_unix_ms();
    match telemetry.gpus.iter_mut().find(|gpu| gpu.gpu_index == sample.gpu_index) {
        Some(existing) => {
            sample.processes = std::mem::take(&mut existing.processes);
            *existing = sample;
        }
        None => telemetry.gpus.push(sample),
    }
}

/// Replaces every sampled GPU's process list with this query's rows.
fn store_gpu_processes(state: &NodeState, processes: &[(u32, ProcessVram)]) {
    let mut telemetry = lock_telemetry(state);
    for gpu in telemetry.gpus.iter_mut() {
        gpu.processes = processes
            .iter()
            .filter(|(gpu_index, _)| *gpu_index == gpu.gpu_index)
            .map(|(_, process)| *process)
            .collect();
    }
}

/// The reader is gone, so no GPU sample is current: telemetry shows them unavailable until it is back.
fn mark_gpus_unavailable(state: &NodeState) {
    lock_telemetry(state).gpus.clear();
}

fn store_memory(state: &NodeState, ram_used_bytes: u64, ram_total_bytes: u64) {
    let mut telemetry = lock_telemetry(state);
    telemetry.sampled_unix_ms = now_unix_ms();
    telemetry.ram_used_bytes = ram_used_bytes;
    telemetry.ram_total_bytes = ram_total_bytes;
}

async fn memory_loop(state: Arc<NodeState>, proc_root: PathBuf) {
    loop {
        if let Some((used, total)) = read_memory_use(&proc_root) {
            store_memory(&state, used, total);
        }
        tokio::time::sleep(Duration::from_secs(TELEMETRY_INTERVAL_SECONDS)).await;
    }
}

/// RAM in use is MemTotal − MemAvailable: what the kernel can't hand out without swapping.
fn read_memory_use(proc_root: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(proc_root.join(MEMORY_INFO_FILE)).ok()?;
    let total = memory_info_bytes(&text, "MemTotal")?;
    let available = memory_info_bytes(&text, "MemAvailable")?;
    Some((total.saturating_sub(available), total))
}

async fn nvidia_reader_loop(state: Arc<NodeState>, nvidia_smi: PathBuf) {
    loop {
        read_nvidia_stream(&state, &nvidia_smi).await;
        mark_gpus_unavailable(&state);
        tokio::time::sleep(Duration::from_secs(READER_RESTART_SECONDS)).await;
    }
}

/// Runs `nvidia-smi ... -l 5` and stores each line it prints, until it exits or can't be started. The child is
/// killed if this task is dropped.
async fn read_nvidia_stream(state: &NodeState, nvidia_smi: &Path) {
    let mut command = tokio::process::Command::new(nvidia_smi);
    command
        .args(NVIDIA_TELEMETRY_QUERY)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(sample) = parse_nvidia_telemetry_line(&line) {
            store_gpu_sample(state, sample);
        }
    }
}

async fn nvidia_process_loop(state: Arc<NodeState>, nvidia_smi: PathBuf, gpus: Vec<GpuAddress>, timeout: Duration) {
    loop {
        if let ToolRun::Printed(text) = run_tool(&nvidia_smi, NVIDIA_PROCESS_QUERY, timeout).await {
            store_gpu_processes(&state, &parse_nvidia_processes(&text, &gpus));
        }
        tokio::time::sleep(Duration::from_secs(TELEMETRY_INTERVAL_SECONDS)).await;
    }
}

async fn rocm_smi_loop(state: Arc<NodeState>, rocm_smi: PathBuf, timeout: Duration) {
    loop {
        match run_tool(&rocm_smi, ROCM_SMI_TELEMETRY_ARGUMENTS, timeout).await {
            ToolRun::Printed(text) => store_rocm_samples(&state, &text),
            _ => mark_gpus_unavailable(&state),
        }
        tokio::time::sleep(Duration::from_secs(TELEMETRY_INTERVAL_SECONDS)).await;
    }
}

/// Replaces the AMD samples with this run's: a card rocm-smi no longer reports becomes unavailable.
fn store_rocm_samples(state: &NodeState, text: &str) {
    let vram_totals = lock_hardware_gpus(state, AMD_VENDOR);
    let samples = parse_rocm_smi_telemetry(text, &vram_totals);
    mark_gpus_unavailable(state);
    for sample in samples {
        store_gpu_sample(state, sample);
    }
}

fn has_gpus_of_vendor(state: &NodeState, vendor: &str) -> bool {
    !lock_hardware_gpus(state, vendor).is_empty()
}

/// Index and total VRAM of the profile's GPUs of one vendor.
fn lock_hardware_gpus(state: &NodeState, vendor: &str) -> Vec<(u32, u64)> {
    let hardware = state.hardware.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    hardware
        .gpus
        .iter()
        .filter(|gpu| gpu.vendor == vendor)
        .map(|gpu| (gpu.index, gpu.vram_bytes))
        .collect()
}

fn lock_telemetry(state: &NodeState) -> std::sync::MutexGuard<'_, NodeTelemetry> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state
        .telemetry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const RECORDED_STREAM: &str =
        include_str!("../../../fixtures/hardware/rtx5080_rtx3090_gfx1036/tools/nvidia-smi-telemetry-stream.csv");
    const RECORDED_PROCESSES: &str =
        include_str!("../../../fixtures/hardware/rtx5080_rtx3090_gfx1036/tools/nvidia-smi-compute-apps.csv");

    fn recorded_addresses() -> Vec<GpuAddress> {
        vec![
            GpuAddress {
                index: 0,
                bus_id: "00000000:01:00.0".to_string(),
            },
            GpuAddress {
                index: 1,
                bus_id: "00000000:0D:00.0".to_string(),
            },
        ]
    }

    #[test]
    fn every_line_of_the_recorded_stream_parses() {
        let samples: Vec<GpuTelemetry> = RECORDED_STREAM
            .lines()
            .map(|line| parse_nvidia_telemetry_line(line).unwrap())
            .collect();
        assert_eq!(samples.len(), 6);
        assert_eq!(samples[0].gpu_index, 0);
        assert_eq!(samples[0].utilization_percent, 22);
        assert_eq!(samples[0].vram_used_bytes, 11881 * BYTES_PER_MIB);
        assert_eq!(samples[0].vram_total_bytes, 16303 * BYTES_PER_MIB);
        assert_eq!(samples[1].gpu_index, 1);
        assert!((samples[1].power_watts - 251.07).abs() < 0.01);
    }

    #[test]
    fn fields_a_gpu_doesnt_report_read_as_zero() {
        let sample = parse_nvidia_telemetry_line("0, [N/A], 100, 200, [N/A], [Not Supported]").unwrap();
        assert_eq!(sample.utilization_percent, 0);
        assert_eq!(sample.power_watts, 0.0);
        assert!(parse_nvidia_telemetry_line("0, 1, lots, 200, 1, 1").is_none());
        assert!(parse_nvidia_telemetry_line("No devices were found").is_none());
    }

    #[test]
    fn recorded_processes_land_on_their_gpus_by_bus_id() {
        let rows = parse_nvidia_processes(RECORDED_PROCESSES, &recorded_addresses());
        assert_eq!(rows.len(), 3);
        let on_second_gpu: Vec<u32> = rows
            .iter()
            .filter(|(index, _)| *index == 1)
            .map(|(_, p)| p.pid)
            .collect();
        assert_eq!(on_second_gpu, [3315816]);
        assert_eq!(rows[1].1.vram_bytes, 9932 * BYTES_PER_MIB);
        assert!(parse_nvidia_processes("1, 00000000:99:00.0, 10", &recorded_addresses()).is_empty());
    }

    // Written from rocm-smi's documented JSON shape, not recorded: no box with ROCm installed was available.
    #[test]
    fn rocm_smi_telemetry_is_read_by_key_words() {
        let text = r#"{"card0": {"GPU use (%)": "37", "GPU Memory Allocated (VRAM%)": "25",
                                  "Temperature (Sensor edge) (C)": "51.0",
                                  "Average Graphics Package Power (W)": "120.0"},
                       "card1": {"Temperature (Sensor edge) (C)": "40.0"},
                       "system": {"PID1234": "python, 0, 1048576, 0, unknown"}}"#;
        let samples = parse_rocm_smi_telemetry(text, &[(0, 8 * 1024 * BYTES_PER_MIB)]);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].gpu_index, 0);
        assert_eq!(samples[0].utilization_percent, 37);
        assert_eq!(samples[0].vram_used_bytes, 2 * 1024 * BYTES_PER_MIB);
        assert!((samples[0].temperature_celsius - 51.0).abs() < 0.01);
        assert!((samples[0].power_watts - 120.0).abs() < 0.01);
        assert!(parse_rocm_smi_telemetry("not json", &[]).is_empty());
    }
}
