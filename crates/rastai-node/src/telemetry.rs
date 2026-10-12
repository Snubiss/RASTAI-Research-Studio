//! GPU and memory telemetry (layout: rust-node-gateway-proto.md section 7.7): one long-running nvidia-smi reader,
//! a per-process VRAM query every 5 s, rocm-smi every 5 s on an AMD node, and RAM from /proc/meminfo. The result
//! is `NodeTelemetry` in `NodeState`, which GetHardware returns and every heartbeat carries (M1-45).
//!
//! A tool that is missing or dies leaves its GPUs out of the telemetry ("unavailable"); leases then fall back to the
//! total VRAM in the hardware profile, and jobs are not affected. A long-running reader that exits, or stays silent
//! for NVIDIA_SILENT_INTERVALS intervals (a hung driver), is stopped, its GPUs become unavailable, and it is started
//! again after READER_RESTART_SECONDS. The node computes on one backend and NVIDIA wins on a box with both
//! (flows/platform.md 1.2), so only that vendor's GPUs are sampled; their indexes then never collide.
//!
//! The nvidia-smi stream keys its rows by nvidia-smi's own `index`, which is safe because detection took the
//! profile's NVIDIA indexes from the same field of the same tool. Every other tool row is matched to a profile GPU by
//! its PCI bus id, found at detection (`GpuAddress`), never by the tool's own numbering: the per-process query names
//! no index, and rocm-smi's `cardN` follows the DRM order, which need not be the kfd order the profile's AMD indexes
//! follow.
//!
//! Memory comes in MiB and is turned into bytes with a checked multiplication: an absurd value from a tool is a
//! garbled row and is skipped, never a panic that would end the reader.
//!
//! Free disk is `rastai_store::free_bytes` on the node's data folder (hardware.proto), sampled with RAM every 5 s into
//! both `NodeTelemetry` and the profile, so GetHardware never shows a start-time value hours later. A failed call
//! stores 0, which reads as "unknown".

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rastai_core::paths::DataPaths;
use rastai_core::time::now_unix_ms;
use rastai_proto::common::v1::{GpuTelemetry, NodeTelemetry, ProcessVram};
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::AbortHandle;

use crate::hardware::{AMD_VENDOR, GpuAddress, HardwareSources, ToolRun, memory_info_bytes, run_tool};
use crate::node::NodeState;

/// Seconds between samples, for every reader (node layout 7.7).
const TELEMETRY_INTERVAL_SECONDS: u64 = 5;
/// A long-running reader that died is started again after this long, so a crashing tool can't spin.
const READER_RESTART_SECONDS: u64 = 30;
/// nvidia-smi's stream prints every interval; this many intervals without a line means it hung, and its last samples
/// would otherwise look current to leases.
const NVIDIA_SILENT_INTERVALS: u64 = 3;
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
    "--showbus",
    "--json",
];
/// The words that find a value in a rocm-smi card object, whose key spelling differs between ROCm releases.
const ROCM_BUS_WORDS: &[&str] = &["pci bus"];
const ROCM_USE_WORDS: &[&str] = &["gpu use"];
const ROCM_VRAM_PERCENT_WORDS: &[&str] = &["vram%"];
const ROCM_TEMPERATURE_WORDS: &[&str] = &["temperature", "edge"];
const ROCM_POWER_WORDS: &[&str] = &["power", "(w)"];
const MEMORY_INFO_FILE: &str = "meminfo";
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// The running telemetry loops. Dropping this stops them, and a stopped reader's tool is killed (kill_on_drop), so
/// a node whose start fails after telemetry began leaves nothing running on the caller's runtime.
pub(crate) struct TelemetryTasks {
    handles: Vec<AbortHandle>,
}

impl Drop for TelemetryTasks {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

/// Starts the telemetry loops, one tokio task each, so a reader sees every background task here. The GPU addresses
/// are detection's: only GPUs a tool described are sampled.
pub(crate) fn start_telemetry(
    state: &Arc<NodeState>,
    sources: &HardwareSources,
    nvidia_gpus: Vec<GpuAddress>,
    amd_gpus: Vec<GpuAddress>,
) -> TelemetryTasks {
    let mut handles = Vec::new();
    let memory = tokio::spawn(memory_and_disk_loop(Arc::clone(state), sources.proc_root.clone()));
    handles.push(memory.abort_handle());
    if !nvidia_gpus.is_empty() {
        let reader = tokio::spawn(nvidia_reader_loop(Arc::clone(state), sources.nvidia_smi.clone()));
        handles.push(reader.abort_handle());
        let processes = tokio::spawn(nvidia_process_loop(
            Arc::clone(state),
            sources.nvidia_smi.clone(),
            nvidia_gpus,
            sources.tool_timeout,
        ));
        handles.push(processes.abort_handle());
    } else if !amd_gpus.is_empty() {
        let rocm = tokio::spawn(rocm_smi_loop(
            Arc::clone(state),
            sources.rocm_smi.clone(),
            amd_gpus,
            sources.tool_timeout,
        ));
        handles.push(rocm.abort_handle());
    }
    TelemetryTasks { handles }
}

/// One line of nvidia-smi's telemetry query. Fields a GPU doesn't report (`[N/A]`, `[Not Supported]`) read as 0;
/// the index and memory must parse and the memory must fit in bytes, else the line is skipped.
fn parse_nvidia_telemetry_line(line: &str) -> Option<GpuTelemetry> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() != NVIDIA_TELEMETRY_FIELDS {
        return None;
    }
    let vram_used_bytes = mib_to_bytes(fields[2])?;
    let vram_total_bytes = mib_to_bytes(fields[3])?;
    Some(GpuTelemetry {
        gpu_index: fields[0].parse().ok()?,
        utilization_percent: fields[1].parse().unwrap_or(0),
        vram_used_bytes,
        vram_total_bytes,
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
            let process = ProcessVram {
                pid: fields[0].parse().ok()?,
                vram_bytes: mib_to_bytes(fields[2])?,
            };
            Some((gpu.index, process))
        })
        .collect()
}

/// A MiB count printed by a tool, in bytes. None when it isn't a number or is too large for bytes in a u64, which
/// only a garbled answer prints.
fn mib_to_bytes(text: &str) -> Option<u64> {
    let mib: u64 = text.parse().ok()?;
    mib.checked_mul(BYTES_PER_MIB)
}

/// rocm-smi's telemetry JSON, one object per card. Each card is matched to its GPU by its `PCI Bus` (`--showbus`)
/// against the kfd addresses detection found; a card on another bus is skipped. rocm-smi reports memory in percent
/// of VRAM, so the bytes come from the profile's totals (`vram_totals`: index and bytes). Per-process rows
/// (`--showpids`) are asked for but not read yet: their layout needs a recording from a ROCm box (M10's AMD runner).
fn parse_rocm_smi_telemetry(text: &str, gpus: &[GpuAddress], vram_totals: &[(u32, u64)]) -> Vec<GpuTelemetry> {
    let Ok(Value::Object(cards)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut samples = Vec::new();
    for card in cards.values() {
        let Some(fields) = card.as_object() else {
            continue;
        };
        if let Some(sample) = parse_rocm_card(fields, gpus, vram_totals) {
            samples.push(sample);
        }
    }
    samples
}

/// One rocm-smi card object as a sample of the GPU on its bus. None when its bus isn't one of `gpus` or its use
/// can't be read; the other fields read as 0 when missing.
fn parse_rocm_card(
    fields: &Map<String, Value>,
    gpus: &[GpuAddress],
    vram_totals: &[(u32, u64)],
) -> Option<GpuTelemetry> {
    let bus = rocm_text(fields, ROCM_BUS_WORDS)?.trim();
    let gpu = gpus.iter().find(|gpu| gpu.bus_id.eq_ignore_ascii_case(bus))?;
    let utilization = rocm_number(fields, ROCM_USE_WORDS)?;
    let mut vram_total_bytes = 0;
    for (index, total) in vram_totals {
        if *index == gpu.index {
            vram_total_bytes = *total;
        }
    }
    let vram_percent = rocm_number(fields, ROCM_VRAM_PERCENT_WORDS).unwrap_or(0.0);
    Some(GpuTelemetry {
        gpu_index: gpu.index,
        utilization_percent: utilization as u32,
        vram_used_bytes: (vram_total_bytes as f64 * vram_percent / 100.0) as u64,
        vram_total_bytes,
        temperature_celsius: rocm_number(fields, ROCM_TEMPERATURE_WORDS).unwrap_or(0.0) as f32,
        power_watts: rocm_number(fields, ROCM_POWER_WORDS).unwrap_or(0.0) as f32,
        processes: Vec::new(),
    })
}

/// The text of the first key that holds every one of `words`, compared without case.
fn rocm_text<'a>(fields: &'a Map<String, Value>, words: &[&str]) -> Option<&'a str> {
    for (key, value) in fields {
        let key = key.to_ascii_lowercase();
        if words.iter().all(|word| key.contains(word)) {
            return value.as_str();
        }
    }
    None
}

fn rocm_number(fields: &Map<String, Value>, words: &[&str]) -> Option<f64> {
    rocm_text(fields, words)?.trim().parse().ok()
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

/// Free bytes in the node's data folder, or 0 ("unknown") when the volume can't be asked.
pub(crate) fn data_folder_free_bytes(paths: &DataPaths) -> u64 {
    rastai_store::disk_space::free_bytes(&paths.data).unwrap_or(0)
}

/// The profile carries free disk too, and GetHardware returns both, so the two are kept the same.
fn store_disk_free(state: &NodeState, disk_free_bytes: u64) {
    lock_telemetry(state).disk_free_bytes = disk_free_bytes;
    state
        .hardware
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .disk_free_bytes = disk_free_bytes;
}

async fn memory_and_disk_loop(state: Arc<NodeState>, proc_root: PathBuf) {
    loop {
        if let Some((used, total)) = read_memory_use(&proc_root) {
            store_memory(&state, used, total);
        }
        store_disk_free(&state, data_folder_free_bytes(&state.paths));
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
    let silence_limit = Duration::from_secs(TELEMETRY_INTERVAL_SECONDS * NVIDIA_SILENT_INTERVALS);
    loop {
        read_nvidia_stream(&state, &nvidia_smi, silence_limit).await;
        mark_gpus_unavailable(&state);
        tokio::time::sleep(Duration::from_secs(READER_RESTART_SECONDS)).await;
    }
}

/// Runs `nvidia-smi ... -l 5` and stores each line it prints, until it exits, can't be started, or prints nothing
/// for `silence_limit`. The child is killed when this returns or the task is dropped (kill_on_drop).
async fn read_nvidia_stream(state: &NodeState, nvidia_smi: &Path, silence_limit: Duration) {
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
    // An exit, a read error or the silence limit all end the loop; the caller then marks the GPUs unavailable.
    while let Ok(Ok(Some(line))) = tokio::time::timeout(silence_limit, lines.next_line()).await {
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

async fn rocm_smi_loop(state: Arc<NodeState>, rocm_smi: PathBuf, gpus: Vec<GpuAddress>, timeout: Duration) {
    loop {
        match run_tool(&rocm_smi, ROCM_SMI_TELEMETRY_ARGUMENTS, timeout).await {
            ToolRun::Printed(text) => store_rocm_samples(&state, &text, &gpus),
            _ => mark_gpus_unavailable(&state),
        }
        tokio::time::sleep(Duration::from_secs(TELEMETRY_INTERVAL_SECONDS)).await;
    }
}

/// Replaces the AMD samples with this run's: a card rocm-smi no longer reports becomes unavailable.
fn store_rocm_samples(state: &NodeState, text: &str, gpus: &[GpuAddress]) {
    let vram_totals = lock_hardware_gpus(state, AMD_VENDOR);
    let samples = parse_rocm_smi_telemetry(text, gpus, &vram_totals);
    mark_gpus_unavailable(state);
    for sample in samples {
        store_gpu_sample(state, sample);
    }
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

    /// One sample line, then silence: a hung nvidia-smi. `exec` makes sleep the process the reader kills.
    #[cfg(unix)]
    const SILENT_NVIDIA_SMI: &str = "#!/bin/sh\necho '0, 22, 11881, 16303, 45, 120.5'\nexec sleep 600\n";
    #[cfg(unix)]
    const SHORT_SILENCE_LIMIT: Duration = Duration::from_millis(300);
    /// Far past the short limit: a reader still running then never noticed the silence.
    #[cfg(unix)]
    const SILENCE_TEST_WAIT: Duration = Duration::from_secs(10);
    /// The memory loop samples at once, so this is only slack for a loaded machine.
    #[cfg(unix)]
    const DISK_SAMPLE_WAIT: Duration = Duration::from_secs(10);
    #[cfg(unix)]
    const DISK_SAMPLE_POLL: Duration = Duration::from_millis(20);

    /// A fresh folder, removed when the test ends, also when an assertion fails.
    #[cfg(unix)]
    struct TestFolder {
        path: PathBuf,
    }

    #[cfg(unix)]
    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A node state in `folder`, with a profile of no GPUs and no samples yet.
    #[cfg(unix)]
    fn test_state(folder: &Path) -> NodeState {
        use rastai_core::paths::DataPaths;
        use rastai_core::settings::{default_settings, node_settings, resolve_settings};
        use rastai_proto::common::v1::HardwareProfile;

        use crate::hardware::system_hardware_sources;
        use crate::identity::load_or_create_identity;
        use crate::node::{NodeMode, NodeOptions, build_node_state};

        let options = NodeOptions {
            mode: NodeMode::Embedded,
            paths: DataPaths {
                config: folder.join("config"),
                data: folder.join("data"),
                cache: folder.join("cache"),
                state: folder.join("state"),
                runtime: folder.join("run"),
            },
            settings: node_settings(&resolve_settings(&[default_settings()])),
            hardware_sources: system_hardware_sources(),
            jobs: crate::jobs::supervisor::default_job_options(Vec::new()),
        };
        let identity = load_or_create_identity(&folder.join("node.key")).unwrap();
        build_node_state(options, identity, HardwareProfile::default())
    }

    // A hung nvidia-smi must not leave its last samples looking current: the reader gives up after the silence.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stream_that_goes_silent_is_stopped() {
        use crate::hardware::write_stand_in_tool;

        let folder = TestFolder {
            path: std::env::temp_dir().join(format!("rnts-{}", std::process::id())),
        };
        let _ = std::fs::remove_dir_all(&folder.path);
        std::fs::create_dir_all(&folder.path).unwrap();
        let script = folder.path.join("nvidia-smi");
        write_stand_in_tool(&script, SILENT_NVIDIA_SMI);
        let state = test_state(&folder.path);
        let reading = read_nvidia_stream(&state, &script, SHORT_SILENCE_LIMIT);
        let finished = tokio::time::timeout(SILENCE_TEST_WAIT, reading).await;
        assert!(finished.is_ok(), "the reader kept waiting on a silent nvidia-smi");
        assert_eq!(lock_telemetry(&state).gpus.len(), 1);
    }

    // Free disk is sampled on every tick into both the telemetry and the profile (hardware.proto), not only at start.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_memory_tick_samples_free_disk_into_telemetry_and_profile() {
        let folder = TestFolder {
            path: std::env::temp_dir().join(format!("rntd-{}", std::process::id())),
        };
        let _ = std::fs::remove_dir_all(&folder.path);
        std::fs::create_dir_all(folder.path.join("data")).unwrap();
        let state = Arc::new(test_state(&folder.path));
        // Start from "unknown", as a node whose data folder appeared after it started.
        store_disk_free(&state, 0);
        let sampling = tokio::spawn(memory_and_disk_loop(Arc::clone(&state), folder.path.join("proc")));
        let started = std::time::Instant::now();
        while lock_telemetry(&state).disk_free_bytes == 0 && started.elapsed() < DISK_SAMPLE_WAIT {
            tokio::time::sleep(DISK_SAMPLE_POLL).await;
        }
        sampling.abort();
        let telemetry_free = lock_telemetry(&state).disk_free_bytes;
        let profile_free = state.hardware.lock().unwrap().disk_free_bytes;
        assert!(telemetry_free > 0, "no free disk in the telemetry");
        assert_eq!(profile_free, telemetry_free);
    }

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

    // u64::MAX MiB can't be counted in bytes: the line is garbled and skipped, and the reader keeps going.
    #[test]
    fn an_overflowing_memory_value_skips_the_line() {
        let too_large = u64::MAX / BYTES_PER_MIB + 1;
        let used_overflows = format!("0, 22, {too_large}, 16303, 45, 120.5");
        let total_overflows = format!("0, 22, 11881, {too_large}, 45, 120.5");
        assert!(parse_nvidia_telemetry_line(&used_overflows).is_none());
        assert!(parse_nvidia_telemetry_line(&total_overflows).is_none());
        let largest = u64::MAX / BYTES_PER_MIB;
        let sample = parse_nvidia_telemetry_line(&format!("0, 22, {largest}, {largest}, 45, 120.5")).unwrap();
        assert_eq!(sample.vram_total_bytes, largest * BYTES_PER_MIB);
    }

    #[test]
    fn an_overflowing_process_row_is_skipped_and_the_rest_kept() {
        let too_large = u64::MAX / BYTES_PER_MIB + 1;
        let text = format!("100, 00000000:01:00.0, {too_large}\n200, 00000000:0D:00.0, 10\n");
        let rows = parse_nvidia_processes(&text, &recorded_addresses());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.pid, 200);
        assert_eq!(rows[0].1.vram_bytes, 10 * BYTES_PER_MIB);
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

    /// Two AMD GPUs in kfd order: index 0 on bus 17, index 1 on bus 03.
    fn amd_addresses() -> Vec<GpuAddress> {
        vec![
            GpuAddress {
                index: 0,
                bus_id: "0000:17:00.0".to_string(),
            },
            GpuAddress {
                index: 1,
                bus_id: "0000:03:00.0".to_string(),
            },
        ]
    }

    // Written from rocm-smi's documented JSON shape, not recorded: no box with ROCm installed was available. card0
    // sits on bus 03, which kfd numbers 1, so its sample must land on GPU 1 with GPU 1's VRAM total.
    #[test]
    fn rocm_smi_cards_are_matched_to_gpus_by_bus() {
        let text = r#"{"card0": {"GPU use (%)": "37", "GPU Memory Allocated (VRAM%)": "25",
                                  "Temperature (Sensor edge) (C)": "51.0",
                                  "Average Graphics Package Power (W)": "120.0", "PCI Bus": "0000:03:00.0"},
                       "card1": {"GPU use (%)": "5", "PCI Bus": "0000:17:00.0"},
                       "card2": {"GPU use (%)": "9", "PCI Bus": "0000:99:00.0"},
                       "card3": {"Temperature (Sensor edge) (C)": "40.0", "PCI Bus": "0000:17:00.0"},
                       "system": {"PID1234": "python, 0, 1048576, 0, unknown"}}"#;
        let vram_totals = [(0, 2 * 1024 * BYTES_PER_MIB), (1, 8 * 1024 * BYTES_PER_MIB)];
        let samples = parse_rocm_smi_telemetry(text, &amd_addresses(), &vram_totals);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].gpu_index, 1);
        assert_eq!(samples[0].utilization_percent, 37);
        assert_eq!(samples[0].vram_total_bytes, 8 * 1024 * BYTES_PER_MIB);
        assert_eq!(samples[0].vram_used_bytes, 2 * 1024 * BYTES_PER_MIB);
        assert!((samples[0].temperature_celsius - 51.0).abs() < 0.01);
        assert!((samples[0].power_watts - 120.0).abs() < 0.01);
        assert_eq!(samples[1].gpu_index, 0);
        assert_eq!(samples[1].utilization_percent, 5);
        assert!(parse_rocm_smi_telemetry("not json", &amd_addresses(), &[]).is_empty());
    }
}
