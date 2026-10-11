//! The compiled default layer of the settings (contracts 11.1, 11.2): the lowest layer, holding every key, so a
//! fresh install with no settings file resolves every setting. `settings::default_settings()` wraps it in a layer.
//!
//! Values the contracts give are copied from 11.2. Where 11.2 names a key without a default, the value here says
//! "none chosen": `null` for a key whose absence turns a feature off or hands the choice to code elsewhere (no
//! quota, the Hugging Face cache's own folder, the seed of each run), `""` for a name nobody picked yet (adapter,
//! model, judge) and `[]` for a list. Keys marked for a later milestone in 11.2 arrive with it: the `archive`
//! section (M11), `eval.judge_min_agreement` and `eval.judge_min_labels` (M5), `plugins.sources` (Phase 2) and the
//! `storage.db_snapshot_*` keys (L2).

use serde_json::{Map, Value, json};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Held back on every GPU for the driver and the desktop (ADR-0010). 11.2 gives no number; one GiB covers a
/// desktop session's own use of the card.
const VRAM_RESERVE_BYTES: u64 = GIB;
/// Room the storage code always leaves free on the data disk (contracts 11.2).
const FREE_SPACE_MARGIN_BYTES: u64 = 5 * GIB;
const SANDBOX_RAM_BYTES: u64 = 4 * GIB;
const SANDBOX_DISK_BYTES: u64 = 10 * GIB;
/// The log rotates at this size and keeps this many old files. 11.2 gives no numbers; these bound the logs of
/// one process to about 60 MiB.
const LOG_MAX_FILE_BYTES: u64 = 10 * MIB;
const LOG_KEEP_FILES: u64 = 5;

/// Every section and key with its default, as one JSON object (`{"ui": {"theme": "system", ...}, ...}`).
pub fn default_values() -> Value {
    let sections = [
        ("ui", ui_defaults()),
        ("daemon", daemon_defaults()),
        ("gateway", gateway_defaults()),
        ("inference", inference_defaults()),
        ("psychology", psychology_defaults()),
        ("training", training_defaults()),
        ("eval", eval_defaults()),
        ("compute", compute_defaults()),
        ("nodes", nodes_defaults()),
        ("storage", storage_defaults()),
        ("hf", hf_defaults()),
        ("plugins", plugins_defaults()),
        ("privacy", privacy_defaults()),
        ("sandbox", sandbox_defaults()),
        ("logging", logging_defaults()),
        ("updates", updates_defaults()),
    ];
    let mut values = Map::new();
    for (name, section) in sections {
        values.insert(name.to_string(), section);
    }
    Value::Object(values)
}

fn ui_defaults() -> Value {
    // `advanced` in the prototype; the strings are English until locales arrive (Phase 2).
    json!({
        "mode": "advanced",
        "theme": "system",
        "density": "comfortable",
        "locale": "en",
        "reduced_motion": false,
        "colorblind_palette": false,
    })
}

fn daemon_defaults() -> Value {
    json!({
        "port": 47820,
        "listen": "127.0.0.1",
        "open_browser": true,
        "bus_buffer_events": 10000,
    })
}

fn gateway_defaults() -> Value {
    json!({
        "enabled": true,
        "retention_days": 30,
        "incognito_default": false,
        "redact_pii_on_export": true,
        "default_adapter": "",
    })
}

fn inference_defaults() -> Value {
    json!({
        "default_runtime": "inspectable",
        "default_model": "",
        "sampling": {
            "temperature": 0.7,
            "top_k": 64,
            "top_p": 0.95,
            "repetition_penalty": 1.0,
            "max_tokens": 1024,
            "seed": null,
        },
        "pin_attention_kernel_for_replay": true,
    })
}

fn psychology_defaults() -> Value {
    // No SAE layers until a model's SAE release names them; the summary layer's k falls back to the constant
    // SUMMARY_FEATURE_TOP_K when unset.
    json!({
        "live_panel": false,
        "overhead_budget_percent": 35,
        "sae_layers_default": [],
        "top_k": 16,
        "summary_layer_top_k": null,
        "sae_fit_max_fvu_increase": 0.05,
    })
}

fn training_defaults() -> Value {
    // memory_cap_bytes null: no cap beyond the GPU's free memory.
    json!({
        "anchors_default": true,
        "answer_only_loss": true,
        "gradient_checkpointing": true,
        "lora": { "rank": 16, "alpha": 32 },
        "checkpoint": { "keep_last": 3, "interval_minutes": 15, "preemption_budget_seconds": 25 },
        "memory_cap_bytes": null,
        "auto_resume": true,
        "rlhf_reward_model_min_accuracy": 0.65,
    })
}

fn eval_defaults() -> Value {
    // An empty judge skips judge tests with RRS-V1001 (flows/training-evaluation.md); an empty drift prompt set
    // means the built-in one.
    json!({
        "drift_prompt_set": "",
        "run_on_new_checkpoint": true,
        "judge_default": "",
    })
}

fn compute_defaults() -> Value {
    // An empty GPU list means any GPU.
    json!({
        "default_node": "local",
        "gpu_assignment": { "chat": [], "training": [] },
        "vram_reserve_bytes": VRAM_RESERVE_BYTES,
        "sandbox_cap": 8,
    })
}

fn nodes_defaults() -> Value {
    json!({
        "heartbeat_seconds": 5,
        "missed_heartbeats_unreachable": 3,
        "ssh_binary": "ssh",
    })
}

fn storage_defaults() -> Value {
    // data_dir null: the data folders of contracts 8.1. quota_bytes null: no quota.
    json!({
        "data_dir": null,
        "quota_bytes": null,
        "gc_interval_hours": 24,
        "free_space_margin_bytes": FREE_SPACE_MARGIN_BYTES,
    })
}

fn hf_defaults() -> Value {
    // cache_dir null: the Hugging Face tools' own cache (paths.rs hf_cache_dir).
    json!({
        "cache_dir": null,
        "offline": false,
    })
}

fn plugins_defaults() -> Value {
    json!({
        "dev_paths": [],
        "experimental_apis": false,
    })
}

fn privacy_defaults() -> Value {
    json!({
        "data_policy": "local-only",
        "crash_reports": false,
        "show_egress_badge": true,
    })
}

fn sandbox_defaults() -> Value {
    // engine_socket "": find Podman, then Docker.
    json!({
        "engine_socket": "",
        "default_caps": {
            "cpu": 2,
            "ram_bytes": SANDBOX_RAM_BYTES,
            "time_seconds": 600,
            "pids_limit": 512,
            "disk_bytes": SANDBOX_DISK_BYTES,
        },
    })
}

fn logging_defaults() -> Value {
    json!({
        "level": "info",
        "max_file_bytes": LOG_MAX_FILE_BYTES,
        "keep_files": LOG_KEEP_FILES,
    })
}

fn updates_defaults() -> Value {
    json!({
        "channel": "stable",
        "auto_check": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The sections of contracts 11.2 built in M1, in its order.
    const M1_SECTIONS: &[&str] = &[
        "ui",
        "daemon",
        "gateway",
        "inference",
        "psychology",
        "training",
        "eval",
        "compute",
        "nodes",
        "storage",
        "hf",
        "plugins",
        "privacy",
        "sandbox",
        "logging",
        "updates",
    ];

    #[test]
    fn every_m1_section_has_defaults() {
        let values = default_values();
        let sections = values.as_object().map(|object| object.len()).unwrap_or(0);
        assert_eq!(sections, M1_SECTIONS.len());
        for section in M1_SECTIONS {
            assert!(values[section].is_object(), "{section}");
        }
    }

    #[test]
    fn contract_defaults_are_copied() {
        let values = default_values();
        assert_eq!(values["ui"]["theme"], "system");
        assert_eq!(values["daemon"]["port"], 47820);
        assert_eq!(values["inference"]["sampling"]["top_k"], 64);
        assert_eq!(values["training"]["lora"]["alpha"], 32);
        assert_eq!(values["storage"]["free_space_margin_bytes"], 5_368_709_120_u64);
        assert_eq!(values["sandbox"]["default_caps"]["ram_bytes"], 4_294_967_296_u64);
    }
}
