//! Settings precedence (contracts 11.1; layout rust-control-plane.md 5.5): defaults < user < project < environment
//! < flags, one key at a time. A later layer overrides the keys it sets and leaves the rest of the section alone.

#![allow(clippy::unwrap_used)]

use rastai_core::settings::{
    ResolvedSettings, SettingsLayer, SettingsSource, default_settings, resolve_settings, setting_bool, setting_text,
    setting_u64, settings_from_environment, settings_from_flags,
};
use serde_json::json;

const PROJECT_FOLDER: &str = "/home/me/demo.rrsproj";

fn variables(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn flags(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| text.to_string()).collect()
}

fn source_of<'a>(resolved: &'a ResolvedSettings, key: &str) -> Option<&'a SettingsSource> {
    resolved
        .sources
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, source)| source)
}

/// All five layers, each setting `ui.theme` and one key of its own in another section.
fn five_layers() -> Vec<SettingsLayer> {
    let user = SettingsLayer {
        source: SettingsSource::User,
        values: json!({"ui": {"theme": "light", "density": "compact"}, "gateway": {"retention_days": 7}}),
    };
    let project = SettingsLayer {
        source: SettingsSource::Project(PROJECT_FOLDER.to_string()),
        values: json!({"ui": {"theme": "dark"}, "privacy": {"data_policy": "trusted-remote"}}),
    };
    let environment = settings_from_environment(&variables(&[
        ("RASTAI_UI__THEME", "acme_midnight"),
        ("RASTAI_DAEMON__PORT", "8899"),
        ("RASTAI_HOME", "/srv/rastai"),
        ("PATH", "/usr/bin"),
    ]));
    let flag_layer = settings_from_flags(&flags(&["ui.theme=light", "hf.offline=true"])).unwrap();
    vec![default_settings(), user, project, environment, flag_layer]
}

#[test]
fn ui_theme_is_system_with_no_file() {
    let resolved = resolve_settings(&[
        default_settings(),
        settings_from_environment(&[]),
        settings_from_flags(&[]).unwrap(),
    ]);
    assert_eq!(setting_text(&resolved, "ui.theme"), "system");
    assert_eq!(source_of(&resolved, "ui.theme"), Some(&SettingsSource::Default));
}

#[test]
fn environment_variable_sets_daemon_port() {
    let environment = settings_from_environment(&variables(&[("RASTAI_DAEMON__PORT", "8899")]));
    assert_eq!(environment.values, json!({"daemon": {"port": 8899}}));
    let resolved = resolve_settings(&[default_settings(), environment]);
    assert_eq!(setting_u64(&resolved, "daemon.port"), 8899);
    assert_eq!(source_of(&resolved, "daemon.port"), Some(&SettingsSource::Environment));
    // The rest of the section keeps its defaults.
    assert_eq!(setting_text(&resolved, "daemon.listen"), "127.0.0.1");
    assert_eq!(source_of(&resolved, "daemon.listen"), Some(&SettingsSource::Default));
}

#[test]
fn each_layer_wins_over_the_ones_below_it_per_key() {
    let layers = five_layers();
    let expectations = [
        ("ui.theme", "light", SettingsSource::Flag),
        ("hf.offline", "true", SettingsSource::Flag),
        ("daemon.port", "8899", SettingsSource::Environment),
        (
            "privacy.data_policy",
            "trusted-remote",
            SettingsSource::Project(PROJECT_FOLDER.to_string()),
        ),
        ("ui.density", "compact", SettingsSource::User),
        ("gateway.retention_days", "7", SettingsSource::User),
        ("ui.mode", "advanced", SettingsSource::Default),
        ("gateway.enabled", "true", SettingsSource::Default),
    ];
    let resolved = resolve_settings(&layers);
    for (key, value, source) in &expectations {
        assert_eq!(setting_text(&resolved, key), *value, "{key}");
        assert_eq!(source_of(&resolved, key), Some(source), "{key}");
    }
    // Without the flags, the environment's theme shows through; without it too, the project's.
    let without_flags = resolve_settings(&layers[..4]);
    assert_eq!(setting_text(&without_flags, "ui.theme"), "acme_midnight");
    let without_environment = resolve_settings(&layers[..3]);
    assert_eq!(setting_text(&without_environment, "ui.theme"), "dark");
    assert_eq!(setting_u64(&without_environment, "daemon.port"), 47820);
}

#[test]
fn nested_keys_override_one_at_a_time() {
    let environment = settings_from_environment(&variables(&[("RASTAI_INFERENCE__SAMPLING__TOP_K", "8")]));
    let flag_layer = settings_from_flags(&flags(&["training.lora.rank=8"])).unwrap();
    let resolved = resolve_settings(&[default_settings(), environment, flag_layer]);
    assert_eq!(setting_u64(&resolved, "inference.sampling.top_k"), 8);
    assert_eq!(resolved.values["inference"]["sampling"]["top_p"], json!(0.95));
    assert_eq!(setting_u64(&resolved, "training.lora.rank"), 8);
    assert_eq!(setting_u64(&resolved, "training.lora.alpha"), 32);
    assert_eq!(
        source_of(&resolved, "training.lora.alpha"),
        Some(&SettingsSource::Default)
    );
}

#[test]
fn every_resolved_key_has_one_source_in_key_order() {
    let resolved = resolve_settings(&five_layers());
    let keys: Vec<&String> = resolved.sources.iter().map(|(key, _)| key).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted);
    assert!(setting_bool(&resolved, "hf.offline"));
}

#[test]
fn a_bad_set_flag_gives_a_diagnostic() {
    for text in [
        "ui.theme",
        "theme=dark",
        "=dark",
        "ui.=dark",
        ".theme=dark",
        "UI.theme=dark",
        "ui.theme-name=dark",
        "ui..theme=dark",
        "ui.theme_=dark",
    ] {
        let refusal = settings_from_flags(&flags(&["daemon.port=8899", text])).unwrap_err();
        assert_eq!(refusal.code, "RRS-C0002", "{text:?}");
    }
}

#[test]
fn a_set_value_is_json_or_plain_text() {
    let layer = settings_from_flags(&flags(&[
        "daemon.port=8899",
        "ui.locale=\"1\"",
        "hf.cache_dir=/data/hf cache",
        "plugins.dev_paths=[\"/src/acme\"]",
        "inference.default_model=",
        "storage.quota_bytes=null",
        "ui.theme=a=b",
    ]))
    .unwrap();
    assert_eq!(
        layer.values,
        json!({
            "daemon": {"port": 8899},
            "ui": {"locale": "1", "theme": "a=b"},
            "hf": {"cache_dir": "/data/hf cache"},
            "plugins": {"dev_paths": ["/src/acme"]},
            "inference": {"default_model": ""},
            "storage": {"quota_bytes": null},
        })
    );
}
