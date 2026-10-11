//! Hardware detection over recorded machines (flows/platform.md 1.2; node layout 7.9). Each case in
//! `fixtures/hardware/` holds what a real box showed: its /proc files, its display devices and kfd nodes from sysfs,
//! and nvidia-smi's answers. `expected.json` beside them is the profile detection must build, with the codes of its
//! notes. Then a hung nvidia-smi: detection gives up after the timeout with RRS-H0002, and the node still starts
//! and answers.
//!
//! Unix only: the stand-in tools are shell scripts, and the node serves a Unix socket.

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod on_unix {
    use std::fs;
    use std::time::{Duration, Instant};

    use rastai_node::hardware::detect_hardware;
    use rastai_proto::common::v1::{GpuInfo, HardwareProfile};
    use rastai_proto::node::v1::GetHardwareRequest;
    use serde_json::Value;

    use crate::common::{
        NvidiaSmi, fixture_folder, machine_from_fixture, new_test_folder, start_node, test_node_options,
    };

    const CASES: &[&str] = &[
        "rtx5080_rtx3090_gfx1036",
        "rtx5080",
        "rtx3090",
        "gfx1036_igpu",
        "no_gpu",
    ];
    /// Long enough for a stand-in script on a loaded machine; the hung-tool tests shorten it to stay quick.
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    const SHORT_TOOL_TIMEOUT: Duration = Duration::from_millis(500);
    /// What a hung tool may add to the timeout before the test calls it blocking: process start and the kill.
    const TIMEOUT_MARGIN: Duration = Duration::from_secs(3);
    const TEST_FOLDER_PREFIX: &str = "rnhw";

    #[tokio::test]
    async fn each_recorded_machine_gives_its_expected_profile() {
        for case in CASES {
            let folder = new_test_folder(TEST_FOLDER_PREFIX);
            let sources = machine_from_fixture(case, &folder, NvidiaSmi::Recorded, TOOL_TIMEOUT);
            let detected = detect_hardware(&sources).await;
            let (expected_profile, expected_notes) = read_expected(case);
            assert_eq!(detected.profile, expected_profile, "{case}");
            let note_codes: Vec<String> = detected.notes.iter().map(|note| note.code.clone()).collect();
            assert_eq!(note_codes, expected_notes, "{case}");
            let _ = fs::remove_dir_all(&folder);
        }
    }

    // The bus ids telemetry maps per-process rows with come from the same recorded query.
    #[tokio::test]
    async fn the_nvidia_bus_ids_are_kept_for_telemetry() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let sources = machine_from_fixture("rtx5080_rtx3090_gfx1036", &folder, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let detected = detect_hardware(&sources).await;
        let bus_ids: Vec<&str> = detected.nvidia_gpus.iter().map(|gpu| gpu.bus_id.as_str()).collect();
        assert_eq!(bus_ids, ["00000000:01:00.0", "00000000:0D:00.0"]);
        let _ = fs::remove_dir_all(&folder);
    }

    // The GPU stays listed from sysfs, with no architecture, so nothing schedules on it (flows/platform.md 1.2).
    #[tokio::test]
    async fn a_hung_nvidia_smi_times_out_with_h0002_and_a_partial_profile() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let sources = machine_from_fixture("rtx5080", &folder, NvidiaSmi::Hung, SHORT_TOOL_TIMEOUT);
        let started = Instant::now();
        let detected = detect_hardware(&sources).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed < SHORT_TOOL_TIMEOUT + TIMEOUT_MARGIN,
            "detection took {elapsed:?}"
        );
        let note_codes: Vec<&str> = detected.notes.iter().map(|note| note.code.as_str()).collect();
        assert_eq!(note_codes, ["RRS-H0002"]);
        assert!(
            detected.notes[0].message.contains("nvidia-smi"),
            "{}",
            detected.notes[0].message
        );
        assert_eq!(detected.profile.gpus.len(), 1);
        assert_eq!(detected.profile.gpus[0].vendor, "nvidia");
        assert_eq!(detected.profile.gpus[0].architecture, "");
        assert_eq!(detected.profile.gpus[0].name, "NVIDIA GPU 10de:2c02");
        assert_eq!(detected.profile.cpu_count, 2);
        let _ = fs::remove_dir_all(&folder);
    }

    // The node's start waits for detection at most one timeout; then it serves, with the GPU listed as unknown.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hung_nvidia_smi_does_not_block_the_node_start() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let sources = machine_from_fixture("rtx5080", &folder, NvidiaSmi::Hung, SHORT_TOOL_TIMEOUT);
        let started = Instant::now();
        let (node, mut client) = start_node(test_node_options(&folder, sources)).await;
        let answer = client.get_hardware(GetHardwareRequest {}).await.unwrap().into_inner();
        let elapsed = started.elapsed();
        assert!(
            elapsed < SHORT_TOOL_TIMEOUT + TIMEOUT_MARGIN,
            "the node answered after {elapsed:?}"
        );
        let hardware = answer.hardware.unwrap();
        assert_eq!(hardware.gpus.len(), 1);
        assert_eq!(hardware.gpus[0].architecture, "");
        node.abort();
        let _ = fs::remove_dir_all(&folder);
    }

    /// The profile and note codes in `fixtures/hardware/<case>/expected.json`.
    fn read_expected(case: &str) -> (HardwareProfile, Vec<String>) {
        let text = fs::read_to_string(fixture_folder(case).join("expected.json")).unwrap();
        let expected: Value = serde_json::from_str(&text).unwrap();
        let profile = HardwareProfile {
            cpu_model: text_field(&expected, "cpu_model"),
            cpu_count: u32::try_from(number_field(&expected, "cpu_count")).unwrap(),
            ram_total_bytes: number_field(&expected, "ram_total_bytes"),
            disk_free_bytes: 0,
            gpus: expected["gpus"].as_array().unwrap().iter().map(expected_gpu).collect(),
            driver_version: text_field(&expected, "driver_version"),
            backend: text_field(&expected, "backend"),
        };
        let notes = expected["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap().to_string())
            .collect();
        (profile, notes)
    }

    fn expected_gpu(value: &Value) -> GpuInfo {
        GpuInfo {
            index: u32::try_from(number_field(value, "index")).unwrap(),
            name: text_field(value, "name"),
            architecture: text_field(value, "architecture"),
            vram_bytes: number_field(value, "vram_bytes"),
            vendor: text_field(value, "vendor"),
            driver_version: text_field(value, "driver_version"),
        }
    }

    fn text_field(value: &Value, key: &str) -> String {
        value[key].as_str().unwrap().to_string()
    }

    fn number_field(value: &Value, key: &str) -> u64 {
        value[key].as_u64().unwrap()
    }
}
