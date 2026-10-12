//! `settings.get` and `settings.set` (contracts 6.4, 11.1), and reading the `settings.json` layers the daemon resolves
//! its settings from.
//!
//! The files are the only store: the user's `<config>/settings.json` and a project's `<project>/settings.json`, each a
//! `rastai.settings/1` document that holds only the keys it sets. `settings.set` reads the file, sets one key, checks
//! the whole document with rastai-formats (an unknown key or a value of the wrong kind is refused with its `RRS-E`
//! problem) and writes it back atomically (`write_config_file`). A file that doesn't parse is never overwritten: it is
//! `RRS-C0009`, and the owner fixes it first. The environment and `--set` layers sit above both files, so a set key may
//! still be overridden; the answer says which layer the value in effect comes from.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use rastai_core::commands::output::CommandOutput;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::paths::user_settings_path;
use rastai_core::settings::{ResolvedSettings, SettingsLayer, SettingsSource, default_settings, resolve_settings};
use rastai_core::versions::APP_VERSION;
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic, diagnostic_at, has_errors};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::commands::result_json;
use rastai_formats::commands::settings::{
    SettingKeySource, SettingsGetArgs, SettingsGetResult, SettingsSetArgs, SettingsSetResult,
};
use rastai_formats::documents::Document;
use rastai_formats::header::header_fields;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{read_document, write_document};
use rastai_store::config_files::write_config_file;
use rastai_store::index::kept_tables::get_project;
use rastai_store::index::reads::open_index_reader;
use serde_json::{Map, Value, json};

use crate::commands::context::{CommandContext, core_failure, store_failure, sync_result};
use crate::server_state::Server;

const SETTINGS_FILE_NAME: &str = "settings.json";
// A new file's header (contracts 5.1). The domain of the schema URL is still open (contracts 14); the fixtures and the
// generated schemas use this one.
const SETTINGS_SCHEMA_URL: &str = "https://schemas.rastai.dev/rastai.settings/1.json";
const SETTINGS_TYPE: &str = "rastai.settings";
const SETTINGS_SCHEMA_VERSION: u32 = 1;
const CREATED_WITH_PREFIX: &str = "rastai ";

const KEY_SEPARATOR: char = '.';
/// `settings.set` writes one key of one section: `section.key`, or deeper (`training.lora.rank`).
const MIN_SET_KEY_SEGMENTS: usize = 2;
const USER_LEVEL: &str = "user";
const PROJECT_LEVEL: &str = "project";

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_SETTINGS_FILE_BROKEN: &str = "RRS-C0009";
const CODE_UNKNOWN_FIELD: &str = "RRS-E1003";
const KEY_PATH: &str = "#/key";
const VALUE_PATH: &str = "#/value";
const LEVEL_PATH: &str = "#/level";
const EXPECTED_KEY: &str = "a section or a dotted key of rastai.settings/1, such as ui or ui.theme";
const EXPECTED_SET_KEY: &str = "a dotted key of rastai.settings/1, such as ui.theme";
const EXPECTED_SETTING: &str = "a key and value that rastai.settings/1 allows";
const EXPECTED_OPEN_PROJECT: &str = "the id of an open project";
const EXPECTED_LEVEL_WITHOUT_PROJECT: &str = "user, as no project is open";

/// The settings file a `settings.set` writes, and the layer it is.
struct SettingsTarget {
    path: PathBuf,
    source: SettingsSource,
}

/// A settings file's layer: its keys without the document header. None for a missing file; `RRS-C0009` for one that
/// can't be read or isn't a valid `rastai.settings/1` document.
pub(crate) fn read_settings_layer(path: &Path, source: SettingsSource) -> Result<Option<SettingsLayer>, Diagnostic> {
    let Some(document) = read_settings_document(path)? else {
        return Ok(None);
    };
    Ok(Some(SettingsLayer {
        source,
        values: without_header(document),
    }))
}

/// The defaults, then the files, then the environment and `--set` layers (contracts 11.1).
pub(crate) fn resolve_with_files(
    user_layer: Option<SettingsLayer>,
    project_layer: Option<SettingsLayer>,
    overrides: &[SettingsLayer],
) -> ResolvedSettings {
    let mut layers = vec![default_settings()];
    layers.extend(user_layer);
    layers.extend(project_layer);
    layers.extend(overrides.iter().cloned());
    resolve_settings(&layers)
}

/// The settings in effect, or the part under one section or key, with the layer each key came from.
pub(crate) async fn get(
    context: &CommandContext<'_>,
    arguments: SettingsGetArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let resolved = settings_for_request(context)?;
    let Some(values) = values_under_key(&resolved.values, &arguments.key) else {
        let refusal = diagnostic_at(
            CODE_INVALID_ARGUMENTS,
            &[("found", &arguments.key), ("expected", EXPECTED_KEY)],
            KEY_PATH,
        );
        return Err(vec![refusal]);
    };
    let result = SettingsGetResult {
        values,
        sources: sources_under_key(&resolved, &arguments.key),
    };
    Ok(sync_result(result_json(&result)))
}

/// Sets one key in the user's or the open project's `settings.json`. A dry run checks the change and answers what
/// would be in effect, and writes nothing.
pub(crate) async fn set(
    context: &CommandContext<'_>,
    arguments: SettingsSetArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let segments = set_key_segments(&arguments.key)?;
    let target = settings_target(context, &arguments.level)?;
    // Held while the file is read, changed and written, so two sets never lose one another's key. Nothing here waits.
    let mut daemon_settings = match context.server.settings.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let document = match read_settings_document(&target.path).map_err(|refusal| vec![refusal])? {
        Some(document) => document,
        None => new_settings_document()?,
    };
    let changed = with_value_at(document, &segments, arguments.value.clone());
    let (bytes, new_layer) = checked_settings_file(&changed, &target.source)?;
    let resolved = resolved_with_new_layer(context, &target, new_layer)?;
    if !context.dry_run {
        write_config_file(&target.path, &bytes).map_err(|error| store_failure(&error))?;
        if target.source == SettingsSource::User {
            *daemon_settings = resolved.clone();
        }
    }
    let result = SettingsSetResult {
        value: value_at(&resolved.values, &arguments.key)
            .cloned()
            .unwrap_or(Value::Null),
        source: source_of_key(&resolved, &arguments.key),
        key: arguments.key,
    };
    Ok(sync_result(result_json(&result)))
}

// ---- Reading the files ----

/// The whole document as JSON, header included, through rastai-formats' four-layer read.
fn read_settings_document(path: &Path) -> Result<Option<Value>, Diagnostic> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(broken_settings_file(path, &error.to_string())),
    };
    let file_name = path.display().to_string();
    let outcome = read_document(&file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
    if let Some(problem) = outcome.diagnostics.iter().find(|item| item.severity == Severity::Error) {
        return Err(broken_settings_file(path, &problem.message));
    }
    match outcome.document {
        Some(Document::Settings(settings)) => match serde_json::to_value(settings) {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(broken_settings_file(path, &error.to_string())),
        },
        _ => Err(broken_settings_file(path, "it isn't a rastai.settings document")),
    }
}

fn broken_settings_file(path: &Path, reason: &str) -> Diagnostic {
    let path_text = path.display().to_string();
    diagnostic(CODE_SETTINGS_FILE_BROKEN, &[("path", &path_text), ("reason", reason)])
}

/// A layer holds settings only; the header keys (`$schema`, `type`, `id`, ...) aren't settings.
fn without_header(document: Value) -> Value {
    let Value::Object(mut members) = document else {
        return document;
    };
    for header_field in header_fields() {
        members.remove(&header_field.name);
    }
    Value::Object(members)
}

/// The daemon's settings, plus the open project's layer when the request names a project.
fn settings_for_request(context: &CommandContext<'_>) -> Result<ResolvedSettings, Vec<Diagnostic>> {
    if context.project_id.is_empty() {
        return Ok(daemon_settings(context.server));
    }
    let project = project_target(context.server, &context.project_id)?;
    let project_layer = read_settings_layer(&project.path, project.source).map_err(|refusal| vec![refusal])?;
    let user_layer = read_user_layer(context.server)?;
    Ok(resolve_with_files(
        user_layer,
        project_layer,
        &context.server.settings_overrides,
    ))
}

fn daemon_settings(server: &Server) -> ResolvedSettings {
    match server.settings.read() {
        Ok(settings) => settings.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn read_user_layer(server: &Server) -> Result<Option<SettingsLayer>, Vec<Diagnostic>> {
    read_settings_layer(&user_settings_path(&server.paths), SettingsSource::User).map_err(|refusal| vec![refusal])
}

// ---- Which file ----

fn settings_target(context: &CommandContext<'_>, level: &str) -> Result<SettingsTarget, Vec<Diagnostic>> {
    if level != PROJECT_LEVEL {
        // Layer 2 already refused any word but `user` and `project`; empty means `user`.
        return Ok(SettingsTarget {
            path: user_settings_path(&context.server.paths),
            source: SettingsSource::User,
        });
    }
    if context.project_id.is_empty() {
        let refusal = diagnostic_at(
            CODE_INVALID_ARGUMENTS,
            &[("found", PROJECT_LEVEL), ("expected", EXPECTED_LEVEL_WITHOUT_PROJECT)],
            LEVEL_PATH,
        );
        return Err(vec![refusal]);
    }
    project_target(context.server, &context.project_id)
}

/// `<project folder>/settings.json` of a project the index knows.
fn project_target(server: &Server, project_id: &str) -> Result<SettingsTarget, Vec<Diagnostic>> {
    // A short single-row read on a pooled connection; it never waits on the writer (WAL).
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    let project = get_project(&reader, project_id).map_err(|error| store_failure(&error))?;
    let Some(project) = project else {
        let refusal = diagnostic(
            CODE_INVALID_ARGUMENTS,
            &[("found", project_id), ("expected", EXPECTED_OPEN_PROJECT)],
        );
        return Err(vec![refusal]);
    };
    Ok(SettingsTarget {
        path: Path::new(&project.path).join(SETTINGS_FILE_NAME),
        source: SettingsSource::Project(project.path),
    })
}

// ---- Changing a file ----

/// `ui.theme` → `["ui", "theme"]`. A key with fewer than two parts, or an empty part, is refused before any file is
/// read.
fn set_key_segments(key: &str) -> Result<Vec<String>, Vec<Diagnostic>> {
    let segments: Vec<String> = key.split(KEY_SEPARATOR).map(str::to_string).collect();
    let well_formed = segments.len() >= MIN_SET_KEY_SEGMENTS && segments.iter().all(|segment| !segment.is_empty());
    if well_formed {
        return Ok(segments);
    }
    let refusal = diagnostic_at(
        CODE_INVALID_ARGUMENTS,
        &[("found", key), ("expected", EXPECTED_SET_KEY)],
        KEY_PATH,
    );
    Err(vec![refusal])
}

/// A settings file for a layer that had none: the header alone (contracts 5.1).
fn new_settings_document() -> Result<Value, Vec<Diagnostic>> {
    let document_id = new_id(IdPrefix::Document).map_err(|error| core_failure(&error))?;
    Ok(json!({
        "$schema": SETTINGS_SCHEMA_URL,
        "type": SETTINGS_TYPE,
        "schema_version": SETTINGS_SCHEMA_VERSION,
        "created_with": format!("{CREATED_WITH_PREFIX}{APP_VERSION}"),
        "id": document_id,
    }))
}

/// The document with `value` at the key, making the sections on the way. A part of the path that holds something
/// other than an object is replaced, and the check that follows says whether that was allowed.
fn with_value_at(document: Value, segments: &[String], value: Value) -> Value {
    let Some((first, rest)) = segments.split_first() else {
        return value;
    };
    let mut members = match document {
        Value::Object(members) => members,
        _ => Map::new(),
    };
    let inner = members.remove(first).unwrap_or(Value::Object(Map::new()));
    let replaced = if rest.is_empty() {
        value
    } else {
        with_value_at(inner, rest, value)
    };
    members.insert(first.clone(), replaced);
    Value::Object(members)
}

/// The changed document checked as a whole (layers 1 to 3) and written in the stable form, with its layer. A problem
/// is `RRS-C0002` at the key (an unknown key) or the value (anything else), then each `RRS-E` problem.
fn checked_settings_file(
    document: &Value,
    source: &SettingsSource,
) -> Result<(Vec<u8>, SettingsLayer), Vec<Diagnostic>> {
    let text = document.to_string();
    let outcome = read_document(
        SETTINGS_FILE_NAME,
        &SliceSource { bytes: text.as_bytes() },
        &ImportLimits::default(),
    );
    let settings_document = match outcome.document {
        Some(document) if !has_errors(&outcome.diagnostics) => document,
        _ => return Err(invalid_setting(outcome.diagnostics)),
    };
    let bytes = write_document(&settings_document).map_err(|error| {
        let reason = format!("{error:?}");
        vec![diagnostic(error.code(), &[("reason", &reason)])]
    })?;
    let layer = SettingsLayer {
        source: source.clone(),
        values: without_header(document.clone()),
    };
    Ok((bytes, layer))
}

fn invalid_setting(findings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let unknown_key = findings.iter().any(|finding| finding.code == CODE_UNKNOWN_FIELD);
    let path = if unknown_key { KEY_PATH } else { VALUE_PATH };
    let summary = diagnostic_at(
        CODE_INVALID_ARGUMENTS,
        &[
            ("found", "a key or value the settings file can't hold"),
            ("expected", EXPECTED_SETTING),
        ],
        path,
    );
    let mut diagnostics = vec![summary];
    diagnostics.extend(findings);
    diagnostics
}

/// The settings that would be in effect with the new layer in place of its file.
fn resolved_with_new_layer(
    context: &CommandContext<'_>,
    target: &SettingsTarget,
    new_layer: SettingsLayer,
) -> Result<ResolvedSettings, Vec<Diagnostic>> {
    let overrides = &context.server.settings_overrides;
    if target.source == SettingsSource::User {
        return Ok(resolve_with_files(Some(new_layer), None, overrides));
    }
    let user_layer = read_user_layer(context.server)?;
    Ok(resolve_with_files(user_layer, Some(new_layer), overrides))
}

// ---- Reading keys ----

fn value_at<'a>(values: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = values;
    for segment in key.split(KEY_SEPARATOR) {
        current = current.get(segment)?;
    }
    Some(current)
}

/// The sections as a settings file holds them, cut down to the one key or section; everything for an empty key.
fn values_under_key(values: &Value, key: &str) -> Option<Value> {
    if key.is_empty() {
        return Some(values.clone());
    }
    let found = value_at(values, key)?.clone();
    let segments: Vec<String> = key.split(KEY_SEPARATOR).map(str::to_string).collect();
    Some(with_value_at(Value::Object(Map::new()), &segments, found))
}

fn sources_under_key(resolved: &ResolvedSettings, key: &str) -> Vec<SettingKeySource> {
    let section_prefix = format!("{key}{KEY_SEPARATOR}");
    resolved
        .sources
        .iter()
        .filter(|(source_key, _)| key.is_empty() || source_key == key || source_key.starts_with(&section_prefix))
        .map(|(source_key, source)| key_source(source_key, source))
        .collect()
}

fn key_source(key: &str, source: &SettingsSource) -> SettingKeySource {
    let project_folder = match source {
        SettingsSource::Project(folder) => folder.clone(),
        _ => String::new(),
    };
    SettingKeySource {
        key: key.to_string(),
        source: source_name(source).to_string(),
        project_folder,
    }
}

/// The layer of a key; for a section, the layer of its first key that a file or override sets, else `default`.
fn source_of_key(resolved: &ResolvedSettings, key: &str) -> String {
    let section_prefix = format!("{key}{KEY_SEPARATOR}");
    let exact = resolved.sources.iter().find(|(source_key, _)| source_key == key);
    let in_section = resolved
        .sources
        .iter()
        .find(|(source_key, source)| source_key.starts_with(&section_prefix) && *source != SettingsSource::Default);
    match exact.or(in_section) {
        Some((_, source)) => source_name(source).to_string(),
        None => source_name(&SettingsSource::Default).to_string(),
    }
}

/// The words `SettingKeySource.source` uses (contracts 6.5).
fn source_name(source: &SettingsSource) -> &'static str {
    match source {
        SettingsSource::Default => "default",
        SettingsSource::User => USER_LEVEL,
        SettingsSource::Project(_) => PROJECT_LEVEL,
        SettingsSource::Environment => "environment",
        SettingsSource::Flag => "flag",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn segments(key: &str) -> Vec<String> {
        set_key_segments(key).unwrap()
    }

    #[test]
    fn set_keys_need_a_section_and_a_key() {
        assert_eq!(segments("ui.theme"), ["ui", "theme"]);
        assert_eq!(segments("training.lora.rank"), ["training", "lora", "rank"]);
        for bad in ["", "ui", "ui.", ".theme", "ui..theme"] {
            let refusal = set_key_segments(bad).unwrap_err();
            assert_eq!(refusal[0].code, CODE_INVALID_ARGUMENTS, "{bad}");
            assert_eq!(refusal[0].path, KEY_PATH, "{bad}");
        }
    }

    #[test]
    fn a_value_is_placed_at_its_key_and_keeps_the_rest() {
        let document = json!({"type": "rastai.settings", "ui": {"density": "compact"}});
        let changed = with_value_at(document, &segments("ui.theme"), json!("dark"));
        assert_eq!(changed["ui"], json!({"density": "compact", "theme": "dark"}));
        assert_eq!(changed["type"], "rastai.settings");
        let deeper = with_value_at(changed, &segments("training.lora.rank"), json!(8));
        assert_eq!(deeper["training"], json!({"lora": {"rank": 8}}));
    }

    #[test]
    fn the_header_is_not_a_layer() {
        let layer = without_header(json!({"$schema": "s", "type": "t", "schema_version": 1, "id": "i", "ui": {}}));
        assert_eq!(layer, json!({"ui": {}}));
    }

    #[test]
    fn a_new_document_with_a_known_key_passes_and_an_unknown_one_is_refused() {
        let document = with_value_at(new_settings_document().unwrap(), &segments("ui.theme"), json!("dark"));
        let (bytes, layer) = checked_settings_file(&document, &SettingsSource::User).unwrap();
        let written: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(written["ui"]["theme"], "dark");
        assert!(written["content_hash"].as_str().unwrap().starts_with("blake3:"));
        assert_eq!(layer.values, json!({"ui": {"theme": "dark"}}));

        let unknown = with_value_at(new_settings_document().unwrap(), &segments("ui.colour"), json!("red"));
        let refusal = checked_settings_file(&unknown, &SettingsSource::User).unwrap_err();
        assert_eq!(
            (refusal[0].code.as_str(), refusal[0].path.as_str()),
            ("RRS-C0002", KEY_PATH)
        );
        assert!(refusal.iter().any(|item| item.code == "RRS-E1003"));

        let wrong_kind = with_value_at(
            new_settings_document().unwrap(),
            &segments("daemon.port"),
            json!("high"),
        );
        let refusal = checked_settings_file(&wrong_kind, &SettingsSource::User).unwrap_err();
        assert_eq!(
            (refusal[0].code.as_str(), refusal[0].path.as_str()),
            ("RRS-C0002", VALUE_PATH)
        );
        assert!(refusal.iter().any(|item| item.code == "RRS-E1002"));
    }

    #[test]
    fn keys_and_sections_are_cut_out_with_their_sources() {
        let user = SettingsLayer {
            source: SettingsSource::User,
            values: json!({"ui": {"theme": "dark"}}),
        };
        let resolved = resolve_with_files(Some(user), None, &[]);
        assert_eq!(
            values_under_key(&resolved.values, "ui.theme").unwrap(),
            json!({"ui": {"theme": "dark"}})
        );
        assert_eq!(values_under_key(&resolved.values, "ui").unwrap()["ui"]["theme"], "dark");
        assert!(values_under_key(&resolved.values, "ui.nothing").is_none());
        assert!(values_under_key(&resolved.values, "nothing").is_none());
        let sources = sources_under_key(&resolved, "ui.theme");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source, "user");
        assert_eq!(source_of_key(&resolved, "ui.theme"), "user");
        assert_eq!(source_of_key(&resolved, "ui"), "user");
        assert_eq!(source_of_key(&resolved, "daemon.port"), "default");
        assert!(sources_under_key(&resolved, "").len() > 10);
    }
}
