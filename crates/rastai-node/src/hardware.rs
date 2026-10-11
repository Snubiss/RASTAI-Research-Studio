//! Hardware detection at every node start (layout: rust-node-gateway-proto.md section 7.9; flows/platform.md 1.2):
//! the display devices in sysfs, then nvidia-smi for NVIDIA GPUs, the kfd topology (and rocm-smi's product names,
//! when it is installed) for AMD GPUs, the CPU and RAM from /proc. This file only reads; the numerical self-test is
//! the `doctor_selftest` job (M2-20), and picking the environment from the profile is environments.rs (M2-16).
//!
//! Tool output is parsed, never trusted: a tool that fails, hangs past DETECTION_TOOL_TIMEOUT_SECONDS or prints
//! something unexpected leaves a partial profile and RRS-H0002 (warning). Its GPUs stay listed from sysfs, with no
//! architecture, so nothing schedules on them. No GPU at all gives RRS-H0001 (info) and the `cpu` backend.
//!
//! GPU indexes are each driver's own (nvidia-smi's index, the kfd node order for AMD), because jobs select GPUs by
//! them (CUDA_VISIBLE_DEVICES, HIP_VISIBLE_DEVICES). A box with both vendors lists both; the node computes on one
//! backend, and NVIDIA wins (flows/platform.md 1.2), so the vendor tells the two index lists apart.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::common::v1::{GpuInfo, HardwareProfile};
use serde_json::Value;

/// A hung nvidia-smi must not block the node's start (flows/platform.md 1.2, contracts 15.10).
pub const DETECTION_TOOL_TIMEOUT_SECONDS: u64 = 5;
const NVIDIA_SMI_PROGRAM: &str = "nvidia-smi";
const ROCM_SMI_PROGRAM: &str = "rocm-smi";
/// The spike's query plus the bus id and compute capability (node layout 7.9).
const NVIDIA_GPU_QUERY: &[&str] = &[
    "--query-gpu=index,pci.bus_id,name,driver_version,memory.total,compute_cap",
    "--format=csv,noheader,nounits",
];
const NVIDIA_GPU_QUERY_FIELDS: usize = 6;
const ROCM_SMI_DETECTION_ARGUMENTS: &[&str] = &["--showproductname", "--showmeminfo", "vram", "--showbus", "--json"];
const NVIDIA_PCI_VENDOR: &str = "0x10de";
const AMD_PCI_VENDOR: &str = "0x1002";
/// PCI class 0x03 is "display controller": VGA, 3D and other display devices.
const DISPLAY_CLASS_PREFIX: &str = "0x03";
pub(crate) const NVIDIA_VENDOR: &str = "nvidia";
pub(crate) const AMD_VENDOR: &str = "amd";
/// The backend of a node without a usable GPU (contracts 8.6 environment ids).
const CPU_BACKEND: &str = "cpu";
const PCI_DEVICES_FOLDER: &str = "bus/pci/devices";
const KFD_NODES_FOLDER: &str = "class/kfd/kfd/topology/nodes";
const KFD_PROPERTIES_FILE: &str = "properties";
const AMD_VRAM_TOTAL_FILE: &str = "mem_info_vram_total";
const CPU_INFO_FILE: &str = "cpuinfo";
const MEMORY_INFO_FILE: &str = "meminfo";
const CPU_MODEL_KEY: &str = "model name";
const CPU_PROCESSOR_KEY: &str = "processor";
/// The keys of a kfd node's `properties` file that detection reads.
const KFD_SIMD_COUNT_KEY: &str = "simd_count";
const KFD_LOCATION_KEY: &str = "location_id";
const KFD_DOMAIN_KEY: &str = "domain";
const KFD_TARGET_VERSION_KEY: &str = "gfx_target_version";
const KFD_VENDOR_ID_KEY: &str = "vendor_id";
const KFD_DEVICE_ID_KEY: &str = "device_id";
const BYTES_PER_MIB: u64 = 1024 * 1024;
const BYTES_PER_KIB: u64 = 1024;

/// Where detection reads from. `system_hardware_sources` is the real machine; tests point it at recorded fixtures
/// and at stand-in tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareSources {
    /// `/sys` on a real machine.
    pub sys_root: PathBuf,
    /// `/proc` on a real machine.
    pub proc_root: PathBuf,
    pub nvidia_smi: PathBuf,
    pub rocm_smi: PathBuf,
    pub tool_timeout: Duration,
}

/// One GPU's profile index and PCI bus id, so telemetry can match a tool's rows to GPUs by bus rather than by the
/// tool's own numbering. NVIDIA's are as nvidia-smi writes them (`00000000:01:00.0`), AMD's as kfd gives them
/// (`0000:17:00.0`, rocm-smi's `PCI Bus` form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAddress {
    pub index: u32,
    pub bus_id: String,
}

/// What detection found: the profile, the notes to show and log (RRS-H0001, RRS-H0002), and the bus ids of each
/// vendor's GPUs that a tool described (none for a GPU listed as unknown).
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedHardware {
    pub profile: HardwareProfile,
    pub notes: Vec<Diagnostic>,
    pub nvidia_gpus: Vec<GpuAddress>,
    pub amd_gpus: Vec<GpuAddress>,
}

/// How a tool run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolRun {
    Printed(String),
    NotInstalled,
    Failed(String),
    TimedOut,
}

/// A display device from sysfs: its PCI address (`0000:01:00.0`), vendor and device ids (`0x10de`, `0x2c02`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayDevice {
    address: String,
    vendor_id: String,
    device_id: String,
}

/// The real machine's sources: /sys, /proc, and the tools on PATH.
pub fn system_hardware_sources() -> HardwareSources {
    HardwareSources {
        sys_root: PathBuf::from("/sys"),
        proc_root: PathBuf::from("/proc"),
        nvidia_smi: PathBuf::from(NVIDIA_SMI_PROGRAM),
        rocm_smi: PathBuf::from(ROCM_SMI_PROGRAM),
        tool_timeout: Duration::from_secs(DETECTION_TOOL_TIMEOUT_SECONDS),
    }
}

/// Detects this machine's hardware. Never fails: whatever can't be read becomes a note and a partial profile. The
/// NVIDIA and AMD probes run at the same time, so two hung tools cost one timeout, not two.
pub async fn detect_hardware(sources: &HardwareSources) -> DetectedHardware {
    let devices = list_display_devices(&sources.sys_root);
    let (nvidia, amd) = tokio::join!(detect_nvidia(sources, &devices), detect_amd(sources, &devices));
    let mut profile = HardwareProfile {
        cpu_model: read_cpu_model(&sources.proc_root),
        cpu_count: read_cpu_count(&sources.proc_root),
        ram_total_bytes: read_memory_total(&sources.proc_root),
        driver_version: nvidia.driver_version.clone(),
        ..HardwareProfile::default()
    };
    profile.gpus.extend(nvidia.gpus);
    profile.gpus.extend(amd.gpus);
    let mut notes = nvidia.notes;
    notes.extend(amd.notes);
    if profile.gpus.is_empty() {
        profile.backend = CPU_BACKEND.to_string();
        notes.push(diagnostic("RRS-H0001", &[]));
    }
    DetectedHardware {
        profile,
        notes,
        nvidia_gpus: nvidia.addresses,
        amd_gpus: amd.addresses,
    }
}

/// Runs a read-only tool with a timeout. A child still running at the timeout is killed (kill_on_drop).
pub(crate) async fn run_tool(program: &Path, arguments: &[&str], timeout: Duration) -> ToolRun {
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ToolRun::NotInstalled,
        Err(error) => return ToolRun::Failed(format!("can't start: {error}")),
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Err(_elapsed) => ToolRun::TimedOut,
        Ok(Err(error)) => ToolRun::Failed(format!("can't be read: {error}")),
        Ok(Ok(output)) if !output.status.success() => ToolRun::Failed(format!("exited with {}", output.status)),
        Ok(Ok(output)) => match String::from_utf8(output.stdout) {
            Ok(text) => ToolRun::Printed(text),
            Err(_) => ToolRun::Failed("printed bytes that aren't UTF-8".to_string()),
        },
    }
}

/// One vendor's part of the profile.
#[derive(Debug, Default)]
struct VendorGpus {
    gpus: Vec<GpuInfo>,
    notes: Vec<Diagnostic>,
    driver_version: String,
    addresses: Vec<GpuAddress>,
}

async fn detect_nvidia(sources: &HardwareSources, devices: &[DisplayDevice]) -> VendorGpus {
    let nvidia_devices: Vec<&DisplayDevice> = devices.iter().filter(|d| d.vendor_id == NVIDIA_PCI_VENDOR).collect();
    if nvidia_devices.is_empty() {
        return VendorGpus::default();
    }
    let parsed = match run_tool(&sources.nvidia_smi, NVIDIA_GPU_QUERY, sources.tool_timeout).await {
        ToolRun::Printed(text) => parse_nvidia_gpus(&text),
        _ => None,
    };
    match parsed {
        Some((gpus, addresses)) => VendorGpus {
            driver_version: gpus.first().map(|gpu| gpu.driver_version.clone()).unwrap_or_default(),
            gpus,
            notes: Vec::new(),
            addresses,
        },
        None => VendorGpus {
            gpus: unknown_gpus(&nvidia_devices, NVIDIA_VENDOR),
            notes: vec![diagnostic("RRS-H0002", &[("tool", NVIDIA_SMI_PROGRAM)])],
            ..VendorGpus::default()
        },
    }
}

/// nvidia-smi's GPU query, one GPU per line; None when any line isn't six well-formed fields.
fn parse_nvidia_gpus(text: &str) -> Option<(Vec<GpuInfo>, Vec<GpuAddress>)> {
    let mut gpus = Vec::new();
    let mut addresses = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() != NVIDIA_GPU_QUERY_FIELDS {
            return None;
        }
        let index: u32 = fields[0].parse().ok()?;
        let memory_mib: u64 = fields[4].parse().ok()?;
        gpus.push(GpuInfo {
            index,
            name: fields[2].to_string(),
            architecture: nvidia_architecture(fields[5])?,
            vram_bytes: memory_mib * BYTES_PER_MIB,
            vendor: NVIDIA_VENDOR.to_string(),
            driver_version: fields[3].to_string(),
        });
        addresses.push(GpuAddress {
            index,
            bus_id: fields[1].to_string(),
        });
    }
    if gpus.is_empty() {
        return None;
    }
    Some((gpus, addresses))
}

/// Compute capability `12.0` → `sm_120`, `8.6` → `sm_86`.
fn nvidia_architecture(compute_capability: &str) -> Option<String> {
    let (major, minor) = compute_capability.split_once('.')?;
    let major: u32 = major.parse().ok()?;
    let minor: u32 = minor.parse().ok()?;
    Some(format!("sm_{major}{minor}"))
}

async fn detect_amd(sources: &HardwareSources, devices: &[DisplayDevice]) -> VendorGpus {
    let amd_devices: Vec<&DisplayDevice> = devices.iter().filter(|d| d.vendor_id == AMD_PCI_VENDOR).collect();
    if amd_devices.is_empty() {
        return VendorGpus::default();
    }
    let kfd_gpus = read_kfd_gpus(&sources.sys_root);
    if kfd_gpus.is_empty() {
        // No kfd node: the amdgpu driver runs the display but offers no compute, so ROCm can't use the GPU.
        return VendorGpus {
            gpus: unknown_gpus(&amd_devices, AMD_VENDOR),
            ..VendorGpus::default()
        };
    }
    let (product_names, notes) = read_rocm_product_names(sources).await;
    let mut gpus = Vec::new();
    let mut addresses = Vec::new();
    for (position, kfd_gpu) in kfd_gpus.iter().enumerate() {
        let gpu = amd_gpu_info(sources, position, kfd_gpu, &product_names);
        addresses.push(GpuAddress {
            index: gpu.index,
            bus_id: kfd_gpu.address.clone(),
        });
        gpus.push(gpu);
    }
    VendorGpus {
        gpus,
        notes,
        addresses,
        ..VendorGpus::default()
    }
}

/// A GPU node of the kfd topology: its PCI address, gfx target and PCI ids.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KfdGpu {
    address: String,
    architecture: String,
    pci_ids: String,
}

fn amd_gpu_info(
    sources: &HardwareSources,
    position: usize,
    kfd_gpu: &KfdGpu,
    product_names: &HashMap<String, String>,
) -> GpuInfo {
    let vram_path = sources
        .sys_root
        .join(PCI_DEVICES_FOLDER)
        .join(&kfd_gpu.address)
        .join(AMD_VRAM_TOTAL_FILE);
    let name = match product_names.get(&kfd_gpu.address) {
        Some(name) => name.clone(),
        None => format!("AMD GPU {}", kfd_gpu.pci_ids),
    };
    GpuInfo {
        index: u32::try_from(position).unwrap_or(u32::MAX),
        name,
        architecture: kfd_gpu.architecture.clone(),
        vram_bytes: read_number_file(&vram_path).unwrap_or(0),
        vendor: AMD_VENDOR.to_string(),
        driver_version: String::new(),
    }
}

/// rocm-smi's product names by PCI address. Not installed is normal (an iGPU on an NVIDIA box) and silent; a
/// rocm-smi that fails or hangs is RRS-H0002, and the names fall back to the PCI ids.
async fn read_rocm_product_names(sources: &HardwareSources) -> (HashMap<String, String>, Vec<Diagnostic>) {
    match run_tool(&sources.rocm_smi, ROCM_SMI_DETECTION_ARGUMENTS, sources.tool_timeout).await {
        ToolRun::NotInstalled => (HashMap::new(), Vec::new()),
        ToolRun::Printed(text) => match parse_rocm_product_names(&text) {
            Some(names) => (names, Vec::new()),
            None => (
                HashMap::new(),
                vec![diagnostic("RRS-H0002", &[("tool", ROCM_SMI_PROGRAM)])],
            ),
        },
        ToolRun::Failed(_) | ToolRun::TimedOut => (
            HashMap::new(),
            vec![diagnostic("RRS-H0002", &[("tool", ROCM_SMI_PROGRAM)])],
        ),
    }
}

/// rocm-smi's JSON: one object per card (`"card0": {"Card Series": ..., "PCI Bus": ...}`). Key spelling differs
/// between ROCm releases ("Card series", "Card Series"), so keys are compared without case. None when the text
/// isn't a JSON object.
fn parse_rocm_product_names(text: &str) -> Option<HashMap<String, String>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let cards = value.as_object()?;
    let mut names = HashMap::new();
    for card in cards.values() {
        let Some(fields) = card.as_object() else {
            continue;
        };
        let field = |wanted: &str| {
            fields
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
                .and_then(|(_, value)| value.as_str())
        };
        if let (Some(bus), Some(series)) = (field("PCI Bus"), field("Card Series")) {
            names.insert(bus.to_ascii_lowercase(), series.to_string());
        }
    }
    Some(names)
}

/// The GPU nodes of the kfd topology in node order, which is the order HIP numbers devices in. CPU nodes have no
/// SIMDs and are skipped.
fn read_kfd_gpus(sys_root: &Path) -> Vec<KfdGpu> {
    let nodes_folder = sys_root.join(KFD_NODES_FOLDER);
    let mut node_numbers: Vec<u32> = list_folder_names(&nodes_folder)
        .iter()
        .filter_map(|name| name.parse().ok())
        .collect();
    node_numbers.sort_unstable();
    node_numbers
        .iter()
        .filter_map(|number| {
            let properties_path = nodes_folder.join(number.to_string()).join(KFD_PROPERTIES_FILE);
            let text = fs::read_to_string(properties_path).ok()?;
            parse_kfd_gpu(&text)
        })
        .collect()
}

/// One kfd node's `properties` file (`key value` lines), as a GPU; None for a CPU node or a file without the keys.
fn parse_kfd_gpu(text: &str) -> Option<KfdGpu> {
    let properties = parse_kfd_properties(text);
    let simd_count = properties.get(KFD_SIMD_COUNT_KEY).copied().unwrap_or(0);
    if simd_count == 0 {
        return None;
    }
    let location = *properties.get(KFD_LOCATION_KEY)?;
    let domain = *properties.get(KFD_DOMAIN_KEY)?;
    let target_version = *properties.get(KFD_TARGET_VERSION_KEY)?;
    let vendor_id = *properties.get(KFD_VENDOR_ID_KEY)?;
    let device_id = *properties.get(KFD_DEVICE_ID_KEY)?;
    let architecture = gfx_target_name(target_version)?;
    Some(KfdGpu {
        address: pci_address(domain, location),
        architecture,
        pci_ids: format!("{vendor_id:04x}:{device_id:04x}"),
    })
}

/// The numeric `key value` lines of a kfd `properties` file. Lines that aren't a key and a number are skipped.
fn parse_kfd_properties(text: &str) -> HashMap<&str, u64> {
    let mut properties = HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        if let Ok(number) = value.trim().parse::<u64>() {
            properties.insert(key, number);
        }
    }
    properties
}

/// kfd's location id is the PCI bus, device and function packed as `bus << 8 | device << 3 | function`.
fn pci_address(domain: u64, location: u64) -> String {
    let bus = (location >> 8) & 0xff;
    let device = (location >> 3) & 0x1f;
    let function = location & 0x7;
    format!("{domain:04x}:{bus:02x}:{device:02x}.{function}")
}

/// kfd's gfx_target_version is `major * 10000 + minor * 100 + stepping`; the target name writes minor and stepping
/// in hex: 100306 → gfx1036, 110000 → gfx1100, 90010 → gfx90a. None for 0 (no target).
fn gfx_target_name(version: u64) -> Option<String> {
    if version == 0 {
        return None;
    }
    let major = version / 10000;
    let minor = (version / 100) % 100;
    let stepping = version % 100;
    Some(format!("gfx{major}{minor:x}{stepping:x}"))
}

/// GPUs a tool couldn't describe: listed from sysfs with their PCI ids and no architecture, so they are never used.
fn unknown_gpus(devices: &[&DisplayDevice], vendor: &str) -> Vec<GpuInfo> {
    devices
        .iter()
        .enumerate()
        .map(|(position, device)| GpuInfo {
            index: u32::try_from(position).unwrap_or(u32::MAX),
            name: format!(
                "{} GPU {}:{}",
                vendor.to_ascii_uppercase(),
                device.vendor_id.trim_start_matches("0x"),
                device.device_id.trim_start_matches("0x")
            ),
            vendor: vendor.to_string(),
            ..GpuInfo::default()
        })
        .collect()
}

/// The display devices on the PCI bus, in address order (nvidia-smi's default order too).
fn list_display_devices(sys_root: &Path) -> Vec<DisplayDevice> {
    let devices_folder = sys_root.join(PCI_DEVICES_FOLDER);
    let mut addresses = list_folder_names(&devices_folder);
    addresses.sort();
    let mut devices = Vec::new();
    for address in addresses {
        if let Some(device) = read_display_device(&devices_folder, address) {
            devices.push(device);
        }
    }
    devices
}

/// The PCI device at `address`, if it is a display device whose ids can be read.
fn read_display_device(devices_folder: &Path, address: String) -> Option<DisplayDevice> {
    let folder = devices_folder.join(&address);
    let class = read_text_file(&folder.join("class"))?;
    if !class.starts_with(DISPLAY_CLASS_PREFIX) {
        return None;
    }
    let vendor_id = read_text_file(&folder.join("vendor"))?;
    let device_id = read_text_file(&folder.join("device"))?;
    Some(DisplayDevice {
        address,
        vendor_id,
        device_id,
    })
}

/// The first `model name` of /proc/cpuinfo (x86), else empty.
fn read_cpu_model(proc_root: &Path) -> String {
    let text = fs::read_to_string(proc_root.join(CPU_INFO_FILE)).unwrap_or_default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() == CPU_MODEL_KEY {
            return value.trim().to_string();
        }
    }
    String::new()
}

/// The logical CPUs: one `processor` line each in /proc/cpuinfo.
fn read_cpu_count(proc_root: &Path) -> u32 {
    let text = fs::read_to_string(proc_root.join(CPU_INFO_FILE)).unwrap_or_default();
    let mut count: u32 = 0;
    for line in text.lines() {
        let key = line.split(':').next().unwrap_or_default();
        if key.trim() == CPU_PROCESSOR_KEY {
            count = count.saturating_add(1);
        }
    }
    count
}

fn read_memory_total(proc_root: &Path) -> u64 {
    let text = fs::read_to_string(proc_root.join(MEMORY_INFO_FILE)).unwrap_or_default();
    memory_info_bytes(&text, "MemTotal").unwrap_or(0)
}

/// A `Key:   12345 kB` line of /proc/meminfo, in bytes.
pub(crate) fn memory_info_bytes(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name.trim() != key {
            return None;
        }
        let kib: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kib * BYTES_PER_KIB)
    })
}

fn list_folder_names(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect()
}

fn read_text_file(path: &Path) -> Option<String> {
    Some(fs::read_to_string(path).ok()?.trim().to_string())
}

fn read_number_file(path: &Path) -> Option<u64> {
    read_text_file(path)?.parse().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const RECORDED_GPUS: &str =
        include_str!("../../../fixtures/hardware/rtx5080_rtx3090_gfx1036/tools/nvidia-smi-gpus.csv");
    const RECORDED_KFD_CPU_NODE: &str =
        include_str!("../../../fixtures/hardware/rtx5080_rtx3090_gfx1036/kfd/0/properties");
    const RECORDED_KFD_GPU_NODE: &str =
        include_str!("../../../fixtures/hardware/rtx5080_rtx3090_gfx1036/kfd/1/properties");

    #[test]
    fn the_recorded_nvidia_query_parses() {
        let (gpus, addresses) = parse_nvidia_gpus(RECORDED_GPUS).unwrap();
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 5080");
        assert_eq!(gpus[0].architecture, "sm_120");
        assert_eq!(gpus[0].vram_bytes, 16303 * BYTES_PER_MIB);
        assert_eq!(gpus[1].architecture, "sm_86");
        assert_eq!(gpus[1].driver_version, "595.91.07");
        assert_eq!(addresses[1].bus_id, "00000000:0D:00.0");
    }

    // Garbage is never half-used: one bad line rejects the whole answer.
    #[test]
    fn a_garbled_nvidia_answer_is_refused_whole() {
        assert!(parse_nvidia_gpus("").is_none());
        assert!(parse_nvidia_gpus("NVIDIA-SMI has failed because it couldn't communicate with the driver").is_none());
        let one_bad_line = format!("{RECORDED_GPUS}2, 00000000:02:00.0, GPU, 1, lots, 8.6\n");
        assert!(parse_nvidia_gpus(&one_bad_line).is_none());
    }

    #[test]
    fn the_recorded_kfd_nodes_give_one_gfx1036_gpu() {
        assert_eq!(parse_kfd_gpu(RECORDED_KFD_CPU_NODE), None);
        let gpu = parse_kfd_gpu(RECORDED_KFD_GPU_NODE).unwrap();
        assert_eq!(gpu.architecture, "gfx1036");
        assert_eq!(gpu.address, "0000:17:00.0");
        assert_eq!(gpu.pci_ids, "1002:164e");
    }

    #[test]
    fn gfx_target_names_write_minor_and_stepping_in_hex() {
        assert_eq!(gfx_target_name(100306).as_deref(), Some("gfx1036"));
        assert_eq!(gfx_target_name(110000).as_deref(), Some("gfx1100"));
        assert_eq!(gfx_target_name(90010).as_deref(), Some("gfx90a"));
        assert_eq!(gfx_target_name(90402).as_deref(), Some("gfx942"));
        assert_eq!(gfx_target_name(0), None);
    }

    // Written from rocm-smi's documented JSON shape, not recorded: no box with ROCm installed was available. Both
    // key spellings seen across ROCm releases are covered.
    #[test]
    fn rocm_product_names_are_read_by_bus() {
        let text = r#"{"card0": {"Card Series": "AMD Radeon RX 7900 XTX", "PCI Bus": "0000:03:00.0"},
                       "card1": {"Card series": "AMD Radeon Graphics", "PCI Bus": "0000:17:00.0"},
                       "system": {"Driver version": "6.8.0"}}"#;
        let names = parse_rocm_product_names(text).unwrap();
        assert_eq!(names["0000:03:00.0"], "AMD Radeon RX 7900 XTX");
        assert_eq!(names["0000:17:00.0"], "AMD Radeon Graphics");
        assert!(parse_rocm_product_names("not json").is_none());
    }

    #[test]
    fn memory_info_lines_are_read_in_bytes() {
        let text = "MemTotal:       64674240 kB\nMemAvailable:   1024 kB\n";
        assert_eq!(memory_info_bytes(text, "MemTotal"), Some(64674240 * 1024));
        assert_eq!(memory_info_bytes(text, "MemAvailable"), Some(1024 * 1024));
        assert_eq!(memory_info_bytes(text, "SwapTotal"), None);
    }
}
