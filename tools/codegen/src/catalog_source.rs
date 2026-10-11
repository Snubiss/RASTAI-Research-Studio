//! Reads the catalog sources (layout rust-control-plane.md 2.6; contracts 10): `catalog/errors/*.json`,
//! `catalog/terms/terms.json`, and each first-party plugin's `plugins/<name>/catalog/errors.json` and `terms.json`.
//!
//! Each file is a JSON array of entries. The files go through rastai-formats' strict reader, so a key written twice
//! in one entry is an error instead of a silent overwrite, and every entry must have exactly the contracts 10 fields.
//! A source that doesn't exist yet holds no entries: the catalog files arrive with M1-13 and the plugins with M4
//! and M5, and the generated tables are simply empty until then.
//!
//! The rules between entries (well-formed codes, no duplicates, each code in its owner's range, nothing deleted,
//! anchors that are slugs used once) are checked by tests/catalog_checks.rs, not here, so this reader stays a reader.

use std::fs;
use std::path::Path;

use rastai_diag::diagnostic::Severity;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde_json::{Map, Value};

use crate::error::{CodegenError, codegen_error};

const ERRORS_FOLDER: &str = "catalog/errors";
const TERMS_FILE: &str = "catalog/terms/terms.json";
const PLUGINS_FOLDER: &str = "plugins";
/// Inside a first-party plugin's folder.
const PLUGIN_ERRORS_FILE: &str = "catalog/errors.json";
const PLUGIN_TERMS_FILE: &str = "catalog/terms.json";
const JSON_EXTENSION: &str = ".json";

/// The fields of an error entry (contracts 10). Every one is required except `fix_command_id`, which may be left out
/// when the entry has no fix button.
const ERROR_FIELDS: [&str; 9] = [
    "code",
    "severity",
    "title",
    "message",
    "basic_text",
    "fix_text",
    "fix_command_id",
    "docs_anchor",
    "retired",
];
const OPTIONAL_ERROR_FIELDS: [&str; 1] = ["fix_command_id"];
/// The fields of a term entry (layout rust-control-plane.md 2.6); all required.
const TERM_FIELDS: [&str; 5] = ["term_id", "title", "basic_text", "advanced_text", "docs_anchor"];

/// One error entry as the catalog JSON holds it, and the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorRecord {
    pub code: String,
    pub severity: Severity,
    pub title: String,
    pub message: String,
    pub basic_text: String,
    pub fix_text: String,
    /// Empty when the entry has no fix button.
    pub fix_command_id: String,
    pub docs_anchor: String,
    pub retired: bool,
    /// Relative to the repository root, with `/` separators: "catalog/errors/RRS-E.json".
    pub source_file: String,
}

/// One plain-English term as the catalog JSON holds it, and the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermRecord {
    pub term_id: String,
    pub title: String,
    pub basic_text: String,
    pub advanced_text: String,
    pub docs_anchor: String,
    pub source_file: String,
}

/// Every error and term entry, sorted by code and by term id so the outputs don't depend on file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogSource {
    pub errors: Vec<ErrorRecord>,
    pub terms: Vec<TermRecord>,
}

/// Reads every catalog source under the repository root.
pub fn read_catalog(repo_root: &Path) -> Result<CatalogSource, CodegenError> {
    let mut catalog = CatalogSource::default();
    for source_file in list_error_files(repo_root)? {
        let bytes = read_source_file(repo_root, &source_file)?;
        catalog.errors.extend(parse_error_file(&bytes, &source_file)?);
    }
    for source_file in list_term_files(repo_root)? {
        let bytes = read_source_file(repo_root, &source_file)?;
        catalog.terms.extend(parse_terms_file(&bytes, &source_file)?);
    }
    catalog.errors.sort_by(|left, right| left.code.cmp(&right.code));
    catalog.terms.sort_by(|left, right| left.term_id.cmp(&right.term_id));
    Ok(catalog)
}

/// Reads the error entries of one file's bytes. `source_file` names the file in errors and in each record.
pub fn parse_error_file(bytes: &[u8], source_file: &str) -> Result<Vec<ErrorRecord>, CodegenError> {
    let entries = read_entries(bytes, source_file)?;
    let mut records = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        records.push(error_record(entry, index, source_file)?);
    }
    Ok(records)
}

/// Reads only the `code` of each entry in one error file's bytes, ignoring every other field. The merge-base check
/// reads the base branch's files with it: those files follow the entry fields of their own day, so reading them
/// with today's strict `parse_error_file` would fail the first PR that changes the fields.
pub fn read_error_codes(bytes: &[u8], source_file: &str) -> Result<Vec<String>, CodegenError> {
    let entries = read_entries(bytes, source_file)?;
    let mut codes = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let place = EntryPlace { index, source_file };
        match entry.get("code") {
            Some(Value::String(code)) => codes.push(code.clone()),
            _ => return Err(entry_error(&place, "field `code` must be a string".to_string())),
        }
    }
    Ok(codes)
}

/// Reads the term entries of one file's bytes.
pub fn parse_terms_file(bytes: &[u8], source_file: &str) -> Result<Vec<TermRecord>, CodegenError> {
    let entries = read_entries(bytes, source_file)?;
    let mut records = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        records.push(term_record(entry, index, source_file)?);
    }
    Ok(records)
}

/// The error files, relative to the repository root: every `catalog/errors/*.json`, then each plugin's
/// `catalog/errors.json`, each group in name order.
pub fn list_error_files(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let mut files = Vec::new();
    for name in list_folder(&repo_root.join(ERRORS_FOLDER))? {
        if name.ends_with(JSON_EXTENSION) {
            files.push(format!("{ERRORS_FOLDER}/{name}"));
        }
    }
    files.extend(list_plugin_files(repo_root, PLUGIN_ERRORS_FILE)?);
    Ok(files)
}

/// The term files: `catalog/terms/terms.json`, then each plugin's `catalog/terms.json`.
pub fn list_term_files(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let mut files = Vec::new();
    if repo_root.join(TERMS_FILE).is_file() {
        files.push(TERMS_FILE.to_string());
    }
    files.extend(list_plugin_files(repo_root, PLUGIN_TERMS_FILE)?);
    Ok(files)
}

fn list_plugin_files(repo_root: &Path, file_in_plugin: &str) -> Result<Vec<String>, CodegenError> {
    let mut files = Vec::new();
    for plugin_name in list_folder(&repo_root.join(PLUGINS_FOLDER))? {
        let source_file = format!("{PLUGINS_FOLDER}/{plugin_name}/{file_in_plugin}");
        if repo_root.join(&source_file).is_file() {
            files.push(source_file);
        }
    }
    Ok(files)
}

/// The names in a folder, sorted. A folder that doesn't exist yet is empty.
fn list_folder(folder: &Path) -> Result<Vec<String>, CodegenError> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let listing = fs::read_dir(folder).map_err(|error| codegen_error(folder, format!("can't list it: {error}")))?;
    let mut names = Vec::new();
    for item in listing {
        let item = item.map_err(|error| codegen_error(folder, format!("can't list it: {error}")))?;
        match item.file_name().into_string() {
            Ok(name) => names.push(name),
            Err(name) => return Err(codegen_error(&folder.join(name), "the name isn't UTF-8".to_string())),
        }
    }
    names.sort();
    Ok(names)
}

fn read_source_file(repo_root: &Path, source_file: &str) -> Result<Vec<u8>, CodegenError> {
    let path = repo_root.join(source_file);
    fs::read(&path).map_err(|error| codegen_error(Path::new(source_file), format!("can't read it: {error}")))
}

/// The entries of a catalog file: a JSON array of objects, read strictly.
fn read_entries(bytes: &[u8], source_file: &str) -> Result<Vec<Map<String, Value>>, CodegenError> {
    let parsed = read_json(bytes, &ImportLimits::default())
        .map_err(|error| source_error(source_file, format!("isn't valid JSON ({}: {error:?})", error.code())))?;
    let Value::Array(items) = parsed.value else {
        return Err(source_error(source_file, "must be a JSON array of entries".to_string()));
    };
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        match item {
            Value::Object(entry) => entries.push(entry),
            _ => {
                return Err(source_error(
                    source_file,
                    format!("entry {index} must be a JSON object"),
                ));
            }
        }
    }
    Ok(entries)
}

fn error_record(entry: &Map<String, Value>, index: usize, source_file: &str) -> Result<ErrorRecord, CodegenError> {
    let place = EntryPlace { index, source_file };
    check_field_names(entry, &ERROR_FIELDS, &OPTIONAL_ERROR_FIELDS, &place)?;
    Ok(ErrorRecord {
        code: text_field(entry, "code", &place)?,
        severity: severity_field(entry, &place)?,
        title: text_field(entry, "title", &place)?,
        message: text_field(entry, "message", &place)?,
        basic_text: text_field(entry, "basic_text", &place)?,
        fix_text: text_field(entry, "fix_text", &place)?,
        fix_command_id: text_field(entry, "fix_command_id", &place)?,
        docs_anchor: text_field(entry, "docs_anchor", &place)?,
        retired: bool_field(entry, "retired", &place)?,
        source_file: source_file.to_string(),
    })
}

fn term_record(entry: &Map<String, Value>, index: usize, source_file: &str) -> Result<TermRecord, CodegenError> {
    let place = EntryPlace { index, source_file };
    check_field_names(entry, &TERM_FIELDS, &[], &place)?;
    Ok(TermRecord {
        term_id: text_field(entry, "term_id", &place)?,
        title: text_field(entry, "title", &place)?,
        basic_text: text_field(entry, "basic_text", &place)?,
        advanced_text: text_field(entry, "advanced_text", &place)?,
        docs_anchor: text_field(entry, "docs_anchor", &place)?,
        source_file: source_file.to_string(),
    })
}

/// Where an entry is, for error messages: its file and its position in the file's array.
struct EntryPlace<'a> {
    index: usize,
    source_file: &'a str,
}

/// Refuses an unknown field (a typo would otherwise be dropped silently) and a missing required one.
fn check_field_names(
    entry: &Map<String, Value>,
    fields: &[&str],
    optional_fields: &[&str],
    place: &EntryPlace,
) -> Result<(), CodegenError> {
    for name in entry.keys() {
        if !fields.contains(&name.as_str()) {
            return Err(entry_error(
                place,
                format!("unknown field `{name}`; the fields are {}", fields.join(", ")),
            ));
        }
    }
    for name in fields {
        if !optional_fields.contains(name) && !entry.contains_key(*name) {
            return Err(entry_error(place, format!("field `{name}` is missing")));
        }
    }
    Ok(())
}

/// A string field. A missing optional field reads as "".
fn text_field(entry: &Map<String, Value>, name: &str, place: &EntryPlace) -> Result<String, CodegenError> {
    match entry.get(name) {
        None => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(entry_error(place, format!("field `{name}` must be a string"))),
    }
}

fn bool_field(entry: &Map<String, Value>, name: &str, place: &EntryPlace) -> Result<bool, CodegenError> {
    match entry.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(entry_error(place, format!("field `{name}` must be true or false"))),
    }
}

fn severity_field(entry: &Map<String, Value>, place: &EntryPlace) -> Result<Severity, CodegenError> {
    match entry.get("severity").and_then(Value::as_str) {
        Some("error") => Ok(Severity::Error),
        Some("warning") => Ok(Severity::Warning),
        Some("info") => Ok(Severity::Info),
        _ => Err(entry_error(
            place,
            "field `severity` must be \"error\", \"warning\" or \"info\"".to_string(),
        )),
    }
}

fn entry_error(place: &EntryPlace, problem: String) -> CodegenError {
    source_error(place.source_file, format!("entry {}: {problem}", place.index))
}

fn source_error(source_file: &str, problem: String) -> CodegenError {
    codegen_error(Path::new(source_file), problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SOURCE_FILE: &str = "catalog/errors/RRS-S.json";
    const DISK_FULL_ENTRY: &str = r#"{
        "code": "RRS-S3001", "severity": "error", "title": "Not enough free disk space",
        "message": "{operation} needs {needed_bytes} bytes.", "basic_text": "Your disk is almost full.",
        "fix_text": "Free up space.", "fix_command_id": "storage.gc", "docs_anchor": "storage-disk-full",
        "retired": false
    }"#;
    const LOGIT_LENS_TERM: &str = r#"{
        "term_id": "logit_lens", "title": "Logit lens", "basic_text": "What each layer would predict.",
        "advanced_text": "Unembedding of each layer's residual stream.", "docs_anchor": "logit-lens"
    }"#;

    fn scratch_repo(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-codegen-source-{}-{test_name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[allow(clippy::unwrap_used)]
    fn write_file(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn problem_of(bytes: &str) -> String {
        match parse_error_file(bytes.as_bytes(), SOURCE_FILE) {
            Ok(records) => format!("unexpectedly read {} records", records.len()),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn an_entry_reads_into_a_record() {
        let records = parse_error_file(format!("[{DISK_FULL_ENTRY}]").as_bytes(), SOURCE_FILE).unwrap();
        assert_eq!(
            records,
            vec![ErrorRecord {
                code: "RRS-S3001".to_string(),
                severity: Severity::Error,
                title: "Not enough free disk space".to_string(),
                message: "{operation} needs {needed_bytes} bytes.".to_string(),
                basic_text: "Your disk is almost full.".to_string(),
                fix_text: "Free up space.".to_string(),
                fix_command_id: "storage.gc".to_string(),
                docs_anchor: "storage-disk-full".to_string(),
                retired: false,
                source_file: SOURCE_FILE.to_string(),
            }]
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn fix_command_id_may_be_left_out() {
        let entry = DISK_FULL_ENTRY.replace(r#""fix_command_id": "storage.gc","#, "");
        let records = parse_error_file(format!("[{entry}]").as_bytes(), SOURCE_FILE).unwrap();
        assert_eq!(records[0].fix_command_id, "");
    }

    #[test]
    fn a_file_that_is_not_an_array_of_objects_is_refused() {
        assert_eq!(
            problem_of(DISK_FULL_ENTRY),
            format!("{SOURCE_FILE}: must be a JSON array of entries")
        );
        assert_eq!(
            problem_of("[1]"),
            format!("{SOURCE_FILE}: entry 0 must be a JSON object")
        );
        assert!(problem_of("[").starts_with(&format!("{SOURCE_FILE}: isn't valid JSON (RRS-E0001")));
    }

    #[test]
    fn a_key_written_twice_is_refused() {
        let entry = DISK_FULL_ENTRY.replace(r#""retired": false"#, r#""retired": false, "retired": true"#);
        assert!(problem_of(&format!("[{entry}]")).starts_with(&format!("{SOURCE_FILE}: isn't valid JSON (RRS-E0003")));
    }

    #[test]
    fn unknown_missing_and_mistyped_fields_are_refused() {
        let unknown = DISK_FULL_ENTRY.replace(r#""retired": false"#, r#""retired": false, "fix": "x""#);
        assert!(problem_of(&format!("[{unknown}]")).contains("entry 0: unknown field `fix`"));
        let missing = DISK_FULL_ENTRY.replace(r#""title": "Not enough free disk space","#, "");
        assert_eq!(
            problem_of(&format!("[{missing}]")),
            format!("{SOURCE_FILE}: entry 0: field `title` is missing")
        );
        let mistyped = DISK_FULL_ENTRY.replace(r#""retired": false"#, r#""retired": "no""#);
        assert!(problem_of(&format!("[{mistyped}]")).ends_with("field `retired` must be true or false"));
        let not_text = DISK_FULL_ENTRY.replace(r#""title": "Not enough free disk space""#, r#""title": 3"#);
        assert!(problem_of(&format!("[{not_text}]")).ends_with("field `title` must be a string"));
    }

    #[test]
    fn an_unknown_severity_is_refused() {
        let entry = DISK_FULL_ENTRY.replace(r#""severity": "error""#, r#""severity": "fatal""#);
        assert!(problem_of(&format!("[{DISK_FULL_ENTRY}, {entry}]")).contains("entry 1: field `severity` must be"));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn codes_read_whatever_the_other_fields_are() {
        let old_style_entry = r#"{"code": "RRS-E0001", "text": "a field this reader has never heard of"}"#;
        let codes = read_error_codes(
            format!("[{DISK_FULL_ENTRY}, {old_style_entry}]").as_bytes(),
            SOURCE_FILE,
        )
        .unwrap();
        assert_eq!(codes, ["RRS-S3001", "RRS-E0001"]);
        let error = read_error_codes(br#"[{"title": "no code"}]"#, SOURCE_FILE).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{SOURCE_FILE}: entry 0: field `code` must be a string")
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn terms_read_into_records_and_refuse_unknown_fields() {
        let records = parse_terms_file(format!("[{LOGIT_LENS_TERM}]").as_bytes(), "catalog/terms/terms.json").unwrap();
        assert_eq!(records[0].term_id, "logit_lens");
        assert_eq!(records[0].advanced_text, "Unembedding of each layer's residual stream.");
        let unknown = LOGIT_LENS_TERM.replace(r#""term_id""#, r#""term""#);
        assert!(parse_terms_file(format!("[{unknown}]").as_bytes(), "terms.json").is_err());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_repo_without_a_catalog_reads_as_empty() {
        let root = scratch_repo("empty");
        fs::create_dir_all(&root).unwrap();
        assert_eq!(read_catalog(&root).unwrap(), CatalogSource::default());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn core_and_plugin_sources_are_read_and_sorted() {
        let root = scratch_repo("full");
        let psychology_entry = DISK_FULL_ENTRY.replace("RRS-S3001", "RRS-L0001");
        let early_entry = DISK_FULL_ENTRY.replace("RRS-S3001", "RRS-S0001");
        write_file(&root, SOURCE_FILE, &format!("[{DISK_FULL_ENTRY}, {early_entry}]"));
        write_file(&root, "catalog/errors/README.md", "not a catalog file");
        write_file(
            &root,
            "plugins/psychology/catalog/errors.json",
            &format!("[{psychology_entry}]"),
        );
        write_file(
            &root,
            "plugins/psychology/catalog/terms.json",
            &format!("[{LOGIT_LENS_TERM}]"),
        );
        write_file(&root, "plugins/evaluation/rastai-plugin.json", "{}");
        write_file(&root, TERMS_FILE, "[]");
        let catalog = read_catalog(&root).unwrap();
        let codes: Vec<&str> = catalog.errors.iter().map(|record| record.code.as_str()).collect();
        assert_eq!(codes, ["RRS-L0001", "RRS-S0001", "RRS-S3001"]);
        assert_eq!(catalog.errors[0].source_file, "plugins/psychology/catalog/errors.json");
        assert_eq!(catalog.terms.len(), 1);
        assert_eq!(catalog.terms[0].source_file, "plugins/psychology/catalog/terms.json");
        fs::remove_dir_all(&root).unwrap();
    }
}
