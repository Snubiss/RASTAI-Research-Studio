//! Settings resolution (contracts 11.1): five layers, lowest to highest — the compiled defaults, the user's
//! `settings.json`, the project's `settings.json`, `RASTAI_<SECTION>__<KEY>` environment variables and
//! `--set <section>.<key>=<value>` flags. A later layer overrides single keys, not whole sections.
//!
//! The files are the only store (spec v5.4 §19): there is no settings table. This module does no file I/O; the
//! daemon reads the two files (rastai-store) and hands their JSON in as layers. Values are checked against the
//! `rastai.settings/1` type elsewhere (rastai-formats), so the getters here never fail: a value of the wrong kind
//! reads as its compiled default, and a bad value can't stop the daemon.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use serde_json::{Map, Value};

use crate::settings_defaults::default_values;

const ENVIRONMENT_PREFIX: &str = "RASTAI_";
/// `RASTAI_INFERENCE__SAMPLING__TOP_K` nests with a double underscore, since keys hold single ones.
const ENVIRONMENT_NESTING: &str = "__";
const KEY_SEPARATOR: char = '.';
const FLAG_VALUE_SEPARATOR: char = '=';
/// A key names at least a section and a key in it.
const MIN_KEY_SEGMENTS: usize = 2;

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const EXPECTED_SET_FLAG: &str = "--set <section>.<key>=<value>, with lowercase snake_case names";

/// The layer a value came from, for `settings.get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSource {
    Default,
    User,
    /// The project folder whose `settings.json` it came from.
    Project(String),
    Environment,
    Flag,
}

/// One layer: a JSON object of sections (`{"daemon": {"port": 8899}}`), holding only the keys it sets.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsLayer {
    pub source: SettingsSource,
    pub values: Value,
}

/// The merged settings, and for each dotted key (`daemon.port`) the layer it came from, sorted by key.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSettings {
    pub values: Value,
    pub sources: Vec<(String, SettingsSource)>,
}

/// What rastai-node reads (compute.*, nodes.*, sandbox.*, storage.*, hf.*). Plain fields, so the node never parses
/// keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSettings {
    /// compute.vram_reserve_bytes
    pub vram_reserve_bytes: u64,
    /// compute.gpu_assignment.chat; empty means any GPU.
    pub chat_gpu_indices: Vec<u32>,
    /// compute.gpu_assignment.training; empty means any GPU.
    pub training_gpu_indices: Vec<u32>,
    /// compute.sandbox_cap
    pub sandbox_cap: u32,
    /// nodes.heartbeat_seconds
    pub heartbeat_seconds: u64,
    /// nodes.missed_heartbeats_unreachable
    pub missed_heartbeats_unreachable: u32,
    /// nodes.ssh_binary
    pub ssh_binary: String,
    /// sandbox.engine_socket; empty means find Podman, then Docker.
    pub sandbox_engine_socket: String,
    /// sandbox.default_caps.cpu
    pub sandbox_default_cpu_count: u32,
    /// sandbox.default_caps.ram_bytes
    pub sandbox_default_ram_bytes: u64,
    /// sandbox.default_caps.time_seconds
    pub sandbox_default_time_seconds: u64,
    /// storage.quota_bytes; 0 means no quota.
    pub storage_quota_bytes: u64,
    /// storage.free_space_margin_bytes
    pub free_space_margin_bytes: u64,
    /// hf.cache_dir; None means `hf_cache_dir(&environment)` in paths.rs.
    pub hf_cache_dir_override: Option<PathBuf>,
    /// hf.offline (`HF_HUB_OFFLINE=1` is honored by the node's download client too).
    pub hf_offline: bool,
}

/// The lowest layer: every key with its compiled default (settings_defaults.rs).
pub fn default_settings() -> SettingsLayer {
    SettingsLayer {
        source: SettingsSource::Default,
        values: default_values(),
    }
}

/// The environment layer from `RASTAI_<SECTION>__<KEY>` variables (`RASTAI_DAEMON__PORT=8899` sets
/// `daemon.port`). The caller passes the process's variables (`std::env::vars()`), so this stays a plain function.
/// Other `RASTAI_` variables (`RASTAI_HOME`, `RASTAI_RUN_ID`) have no `__` and are left alone; a name that isn't
/// upper-case words joined this way is skipped. Values are read as `--set` reads them (`parse_setting_value`).
pub fn settings_from_environment(variables: &[(String, String)]) -> SettingsLayer {
    let mut values = Map::new();
    for (name, text) in variables {
        if let Some(segments) = environment_key_segments(name) {
            insert_at_key(&mut values, &segments, parse_setting_value(text));
        }
    }
    SettingsLayer {
        source: SettingsSource::Environment,
        values: Value::Object(values),
    }
}

/// The flag layer from the text after each `--set`: `section.key=value`, nested keys with more dots
/// (`training.lora.rank=8`). The first malformed flag is refused with `RRS-C0002`; whether the key exists is the
/// settings validation's job, not this parser's.
#[allow(clippy::result_large_err)]
pub fn settings_from_flags(set_flags: &[String]) -> Result<SettingsLayer, Diagnostic> {
    let mut values = Map::new();
    for flag in set_flags {
        let (segments, text) = split_set_flag(flag)?;
        insert_at_key(&mut values, &segments, parse_setting_value(text));
    }
    Ok(SettingsLayer {
        source: SettingsSource::Flag,
        values: Value::Object(values),
    })
}

/// Merges the layers, lowest first. Objects merge key by key; any other value (lists included) replaces what the
/// lower layers had. A layer whose root isn't an object adds nothing: the daemon reports such a file with its
/// `RRS-E` diagnostics and keeps going (contracts 11.1).
pub fn resolve_settings(layers: &[SettingsLayer]) -> ResolvedSettings {
    let mut values = Map::new();
    let mut sources = BTreeMap::new();
    for layer in layers {
        if let Value::Object(layer_values) = &layer.values {
            merge_object(&mut values, layer_values, "", &layer.source, &mut sources);
        }
    }
    ResolvedSettings {
        values: Value::Object(values),
        sources: sources.into_iter().collect(),
    }
}

/// A true/false setting; a missing key or one of another kind reads as its default, else false.
pub fn setting_bool(settings: &ResolvedSettings, key: &str) -> bool {
    match find_value(&settings.values, key).and_then(Value::as_bool) {
        Some(value) => value,
        None => default_value(key).and_then(|value| value.as_bool()).unwrap_or(false),
    }
}

/// A whole-number setting; a missing key or one of another kind (null, negative, fractional) reads as its default,
/// else 0.
pub fn setting_u64(settings: &ResolvedSettings, key: &str) -> u64 {
    match find_value(&settings.values, key).and_then(Value::as_u64) {
        Some(value) => value,
        None => default_value(key).and_then(|value| value.as_u64()).unwrap_or(0),
    }
}

/// A text setting. A number or true/false reads as its text (`RASTAI_UI__LOCALE=1` gives "1"); null, a missing
/// key or a list or object reads as its default, else "".
pub fn setting_text(settings: &ResolvedSettings, key: &str) -> String {
    match find_value(&settings.values, key).and_then(scalar_text) {
        Some(text) => text,
        None => default_value(key)
            .and_then(|value| scalar_text(&value))
            .unwrap_or_default(),
    }
}

/// The node's settings, read once when it starts and again on `settings.set`.
pub fn node_settings(resolved: &ResolvedSettings) -> NodeSettings {
    NodeSettings {
        vram_reserve_bytes: setting_u64(resolved, "compute.vram_reserve_bytes"),
        chat_gpu_indices: setting_index_list(resolved, "compute.gpu_assignment.chat"),
        training_gpu_indices: setting_index_list(resolved, "compute.gpu_assignment.training"),
        sandbox_cap: setting_u32(resolved, "compute.sandbox_cap"),
        heartbeat_seconds: setting_u64(resolved, "nodes.heartbeat_seconds"),
        missed_heartbeats_unreachable: setting_u32(resolved, "nodes.missed_heartbeats_unreachable"),
        ssh_binary: setting_text(resolved, "nodes.ssh_binary"),
        sandbox_engine_socket: setting_text(resolved, "sandbox.engine_socket"),
        sandbox_default_cpu_count: setting_u32(resolved, "sandbox.default_caps.cpu"),
        sandbox_default_ram_bytes: setting_u64(resolved, "sandbox.default_caps.ram_bytes"),
        sandbox_default_time_seconds: setting_u64(resolved, "sandbox.default_caps.time_seconds"),
        storage_quota_bytes: setting_u64(resolved, "storage.quota_bytes"),
        free_space_margin_bytes: setting_u64(resolved, "storage.free_space_margin_bytes"),
        hf_cache_dir_override: setting_optional_path(resolved, "hf.cache_dir"),
        hf_offline: setting_bool(resolved, "hf.offline"),
    }
}

/// A setting's text as JSON when it reads as JSON (`8899`, `true`, `[0, 1]`, `null`, `"8899"`), else as plain
/// text (`dark`, `/home/me/hf`). Quote a text that would otherwise read as JSON.
fn parse_setting_value(text: &str) -> Value {
    match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => Value::String(text.to_string()),
    }
}

/// `DAEMON__PORT` after the prefix -> `["daemon", "port"]`; None for a name that isn't a settings variable.
fn environment_key_segments(name: &str) -> Option<Vec<String>> {
    let rest = name.strip_prefix(ENVIRONMENT_PREFIX)?;
    let segments: Vec<String> = rest.split(ENVIRONMENT_NESTING).map(str::to_ascii_lowercase).collect();
    let all_valid = segments.iter().all(|segment| is_key_segment(segment));
    let upper_case = rest.bytes().all(|byte| !byte.is_ascii_lowercase());
    if segments.len() >= MIN_KEY_SEGMENTS && all_valid && upper_case {
        Some(segments)
    } else {
        None
    }
}

/// `daemon.port=8899` -> (`["daemon", "port"]`, `"8899"`), or the `RRS-C0002` refusal.
#[allow(clippy::result_large_err)]
fn split_set_flag(flag: &str) -> Result<(Vec<String>, &str), Diagnostic> {
    let refusal = || {
        diagnostic(
            CODE_INVALID_ARGUMENTS,
            &[("found", flag), ("expected", EXPECTED_SET_FLAG)],
        )
    };
    let (key, text) = flag.split_once(FLAG_VALUE_SEPARATOR).ok_or_else(refusal)?;
    let segments: Vec<String> = key.split(KEY_SEPARATOR).map(str::to_string).collect();
    if segments.len() < MIN_KEY_SEGMENTS || !segments.iter().all(|segment| is_key_segment(segment)) {
        return Err(refusal());
    }
    Ok((segments, text))
}

/// A section or key name: `[a-z][a-z0-9_]*` without a trailing underscore (`retention_days`, `top_k`).
fn is_key_segment(segment: &str) -> bool {
    let starts_with_letter = segment.bytes().next().is_some_and(|first| first.is_ascii_lowercase());
    let allowed_bytes = segment
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    starts_with_letter && allowed_bytes && !segment.ends_with('_')
}

/// Sets `values[segments...]` to `value`, making sections on the way and replacing a plain value that sits where
/// a section has to go.
fn insert_at_key(values: &mut Map<String, Value>, segments: &[String], value: Value) {
    match segments {
        [] => {}
        [last] => {
            values.insert(last.clone(), value);
        }
        [first, rest @ ..] => {
            let entry = values.entry(first.clone()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(section) = entry {
                insert_at_key(section, rest, value);
            }
        }
    }
}

/// Merges one layer's object into the result at `prefix`, recording the source of each value it sets.
fn merge_object(
    target: &mut Map<String, Value>,
    layer_values: &Map<String, Value>,
    prefix: &str,
    source: &SettingsSource,
    sources: &mut BTreeMap<String, SettingsSource>,
) {
    for (name, value) in layer_values {
        let key = join_key(prefix, name);
        match value {
            Value::Object(nested) => {
                let slot = target.entry(name.clone()).or_insert_with(|| Value::Object(Map::new()));
                if !slot.is_object() {
                    // A lower layer set a plain value where this one has a section of keys.
                    forget_sources(sources, &key);
                    *slot = Value::Object(Map::new());
                }
                if let Value::Object(slot_object) = slot {
                    merge_object(slot_object, nested, &key, source, sources);
                }
            }
            _ => {
                // The value replaces the key and anything a lower layer had nested under it.
                forget_sources(sources, &key);
                target.insert(name.clone(), value.clone());
                sources.insert(key, source.clone());
            }
        }
    }
}

/// Drops the source of `key` and of every key under it.
fn forget_sources(sources: &mut BTreeMap<String, SettingsSource>, key: &str) {
    let nested_prefix = format!("{key}{KEY_SEPARATOR}");
    sources.retain(|known, _| known != key && !known.starts_with(&nested_prefix));
}

fn join_key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}{KEY_SEPARATOR}{name}")
    }
}

/// The value at a dotted key (`inference.sampling.top_k`).
fn find_value<'a>(values: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = values;
    for segment in key.split(KEY_SEPARATOR) {
        current = current.get(segment)?;
    }
    Some(current)
}

/// The compiled default of a key. Built only when a value is missing or of the wrong kind, the rare path.
fn default_value(key: &str) -> Option<Value> {
    find_value(&default_values(), key).cloned()
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// A count that fits in 32 bits; a larger one is capped rather than wrapped.
fn setting_u32(settings: &ResolvedSettings, key: &str) -> u32 {
    u32::try_from(setting_u64(settings, key)).unwrap_or(u32::MAX)
}

/// A list of GPU indices; entries that aren't small whole numbers are skipped.
fn setting_index_list(settings: &ResolvedSettings, key: &str) -> Vec<u32> {
    let Some(Value::Array(entries)) = find_value(&settings.values, key) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(Value::as_u64)
        .filter_map(|index| u32::try_from(index).ok())
        .collect()
}

/// A folder setting; null or "" means none.
fn setting_optional_path(settings: &ResolvedSettings, key: &str) -> Option<PathBuf> {
    let text = setting_text(settings, key);
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layer(source: SettingsSource, values: Value) -> SettingsLayer {
        SettingsLayer { source, values }
    }

    /// The word `settings.get` writes for a layer. The match lists every variant, so a new layer fails to compile
    /// here until rastai-formats' `SettingKeySource.source` words are checked against it.
    fn source_layer_word(source: &SettingsSource) -> &'static str {
        match source {
            SettingsSource::Default => "default",
            SettingsSource::User => "user",
            SettingsSource::Project(_) => "project",
            SettingsSource::Environment => "environment",
            SettingsSource::Flag => "flag",
        }
    }

    // rastai-formats describes `SettingKeySource.source` with its own copy of the layer words, because it can't read
    // this crate; this test keeps the two lists the same, lowest layer first.
    #[test]
    fn the_described_source_words_are_the_layers() {
        let every_source = [
            SettingsSource::Default,
            SettingsSource::User,
            SettingsSource::Project(String::new()),
            SettingsSource::Environment,
            SettingsSource::Flag,
        ];
        let layer_words: Vec<&str> = every_source.iter().map(source_layer_word).collect();
        let key_source = rastai_formats::commands::settings::describe_setting_key_source();
        let source = key_source.fields.iter().find(|field| field.name == "source").unwrap();
        let rastai_formats::describe::types::FieldKind::Enum(described) = &source.kind else {
            panic!("SettingKeySource.source is not an enum: {:?}", source.kind);
        };
        assert_eq!(described, &layer_words);
    }

    #[test]
    fn plain_text_and_json_values() {
        assert_eq!(parse_setting_value("8899"), json!(8899));
        assert_eq!(parse_setting_value("true"), json!(true));
        assert_eq!(parse_setting_value("[0, 1]"), json!([0, 1]));
        assert_eq!(parse_setting_value("dark"), json!("dark"));
        assert_eq!(parse_setting_value("\"8899\""), json!("8899"));
        assert_eq!(parse_setting_value(""), json!(""));
    }

    #[test]
    fn environment_names_map_to_keys() {
        let segments = environment_key_segments("RASTAI_INFERENCE__SAMPLING__TOP_K").unwrap();
        assert_eq!(segments, ["inference", "sampling", "top_k"]);
        for name in [
            "RASTAI_HOME",
            "RASTAI_RUN_ID",
            "RASTAI_DAEMON__",
            "RASTAI___PORT",
            "RASTAI_DAEMON___PORT",
            "RASTAI_daemon__port",
            "RASTAI_DAEMON__PORT-2",
            "HOME",
        ] {
            assert_eq!(environment_key_segments(name), None, "{name}");
        }
    }

    #[test]
    fn a_plain_value_replaces_a_section_and_its_sources() {
        let resolved = resolve_settings(&[
            layer(
                SettingsSource::Default,
                json!({"training": {"lora": {"rank": 16, "alpha": 32}}}),
            ),
            layer(SettingsSource::Flag, json!({"training": {"lora": 5}})),
        ]);
        assert_eq!(resolved.values["training"]["lora"], json!(5));
        assert_eq!(resolved.sources, [("training.lora".to_string(), SettingsSource::Flag)]);
    }

    #[test]
    fn a_section_replaces_a_plain_value() {
        let resolved = resolve_settings(&[
            layer(SettingsSource::Default, json!({"hf": {"cache_dir": null}})),
            layer(SettingsSource::User, json!({"hf": {"cache_dir": {"path": "/x"}}})),
        ]);
        assert_eq!(resolved.values["hf"]["cache_dir"]["path"], "/x");
        assert_eq!(
            resolved.sources,
            [("hf.cache_dir.path".to_string(), SettingsSource::User)]
        );
    }

    #[test]
    fn a_layer_that_is_not_an_object_adds_nothing() {
        let resolved = resolve_settings(&[
            default_settings(),
            layer(SettingsSource::User, json!(["not", "settings"])),
        ]);
        assert_eq!(resolved.values, default_values());
    }

    #[test]
    fn wrong_kinds_read_as_the_default() {
        let resolved = resolve_settings(&[
            default_settings(),
            layer(
                SettingsSource::Environment,
                json!({"daemon": {"port": "abc", "open_browser": 1}}),
            ),
        ]);
        assert_eq!(setting_u64(&resolved, "daemon.port"), 47820);
        assert!(setting_bool(&resolved, "daemon.open_browser"));
        assert_eq!(setting_u64(&resolved, "no.such_key"), 0);
        assert_eq!(setting_text(&resolved, "no.such_key"), "");
    }

    #[test]
    fn node_settings_read_the_defaults() {
        let node = node_settings(&resolve_settings(&[default_settings()]));
        assert_eq!(node.sandbox_cap, 8);
        assert_eq!(node.heartbeat_seconds, 5);
        assert_eq!(node.missed_heartbeats_unreachable, 3);
        assert_eq!(node.ssh_binary, "ssh");
        assert_eq!(node.sandbox_default_cpu_count, 2);
        assert_eq!(node.sandbox_default_time_seconds, 600);
        assert_eq!(node.storage_quota_bytes, 0);
        assert_eq!(node.free_space_margin_bytes, 5 * 1024 * 1024 * 1024);
        assert!(node.chat_gpu_indices.is_empty());
        assert_eq!(node.hf_cache_dir_override, None);
        assert!(!node.hf_offline);
    }

    #[test]
    fn node_settings_follow_overrides() {
        let flags = settings_from_flags(&[
            "compute.gpu_assignment.chat=[0, 1]".to_string(),
            "hf.cache_dir=/data/hf".to_string(),
            "compute.sandbox_cap=99999999999".to_string(),
        ])
        .unwrap();
        let node = node_settings(&resolve_settings(&[default_settings(), flags]));
        assert_eq!(node.chat_gpu_indices, [0, 1]);
        assert_eq!(node.hf_cache_dir_override, Some(PathBuf::from("/data/hf")));
        assert_eq!(node.sandbox_cap, u32::MAX);
    }
}
