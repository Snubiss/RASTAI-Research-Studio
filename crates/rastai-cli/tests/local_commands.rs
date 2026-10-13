//! The commands that need no daemon (layout: rust-control-plane.md 9.7), run as the real binary: `rastai validate`
//! gives exactly the codes and paths of the document corpus (fixtures/documents, contracts 12.5), `version` prints
//! this build's version, and `storage rebuild-index` rebuilds `rastai.db` with the daemon stopped and refuses while
//! one holds the lock (its row-by-row check against a live index is tests/rebuild_index.rs). None of them starts a
//! daemon.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use rastai_core::versions::APP_VERSION;
use serde_json::{Value, json};

#[cfg(target_os = "linux")]
use common::DaemonGuard;
use common::{daemon_processes, final_codes, final_line, run_rastai, run_rastai_in, test_home};

const EXPECTED_SUFFIX: &str = ".expected.json";
const INVALID_FOLDER: &str = "invalid";
const VALID_FOLDER: &str = "valid";

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/documents")
}

/// Each `<type>/<valid or invalid>` folder of the corpus.
fn corpus_folders(kind: &str) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = fs::read_dir(corpus_root())
        .unwrap()
        .map(|entry| entry.unwrap().path().join(kind))
        .filter(|folder| folder.is_dir())
        .collect();
    folders.sort();
    folders
}

/// The fixture files of a folder by name, sorted, without the expected files.
fn fixture_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(EXPECTED_SUFFIX))
        .collect();
    names.sort();
    names
}

/// (code, path) pairs of a `{"diagnostics": [...]}` value.
fn codes_and_paths(value: &Value) -> Vec<(String, String)> {
    let listed = value["diagnostics"].as_array().cloned().unwrap_or_default();
    listed
        .iter()
        .map(|item| {
            (
                item["code"].as_str().unwrap().to_string(),
                item["path"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Runs `rastai validate --json` on every file of a folder, from inside it (the corpus paths are file names), and
/// returns each file's report line by file name, and the exit code.
fn validate_folder(home: &Path, folder: &Path) -> (Vec<(String, Value)>, i32) {
    let names = fixture_names(folder);
    let mut arguments = vec!["validate", "--json"];
    arguments.extend(names.iter().map(String::as_str));
    let result = run_rastai_in(home, folder, &arguments);
    let lines: Vec<Value> = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        names.len() + 1,
        "one line per file and the final line: {result:?}"
    );
    let reports = lines[..names.len()]
        .iter()
        .map(|line| (line["file"].as_str().unwrap().to_string(), line.clone()))
        .collect();
    (reports, result.exit_code)
}

#[test]
fn validate_gives_the_corpus_codes_and_paths_of_every_invalid_file() {
    let home = test_home("validate-invalid");
    let mut checked = 0;
    for folder in corpus_folders(INVALID_FOLDER) {
        let (reports, exit_code) = validate_folder(&home, &folder);
        assert_eq!(exit_code, 1, "{}", folder.display());
        for (file, report) in reports {
            let expected_text = fs::read_to_string(folder.join(format!("{file}{EXPECTED_SUFFIX}"))).unwrap();
            let expected: Value = serde_json::from_str(&expected_text).unwrap();
            assert_eq!(
                codes_and_paths(&report),
                codes_and_paths(&expected),
                "{}/{file}",
                folder.display()
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "only {checked} corpus files were checked");
    assert!(daemon_processes(&home).is_empty(), "validate started a daemon");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn validate_passes_every_valid_file() {
    let home = test_home("validate-valid");
    for folder in corpus_folders(VALID_FOLDER) {
        let (reports, exit_code) = validate_folder(&home, &folder);
        assert_eq!(exit_code, 0, "{}: {reports:?}", folder.display());
        for (file, report) in reports {
            let errors: Vec<&Value> = report["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["severity"] == "error")
                .collect();
            assert!(errors.is_empty(), "{}/{file}: {errors:?}", folder.display());
        }
    }
    let _ = fs::remove_dir_all(home);
}

#[test]
fn validate_in_text_mode_shows_the_code_and_the_line() {
    let home = test_home("validate-text");
    let folder = corpus_root().join("any").join(INVALID_FOLDER);
    let result = run_rastai_in(&home, &folder, &["validate", "nan.json"]);
    assert_eq!(result.exit_code, 1, "{result:?}");
    assert!(result.stdout.contains("error[RRS-E0004]"), "{}", result.stdout);
    assert!(result.stdout.contains("--> nan.json:2:"), "{}", result.stdout);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_file_that_does_not_exist_is_unreadable() {
    let home = test_home("validate-missing");
    let result = run_rastai(&home, &["validate", "--json", "missing.rrsmodel"]);
    assert_eq!(result.exit_code, 1, "{result:?}");
    let report: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(
        codes_and_paths(&report),
        [("RRS-D3001".to_string(), "missing.rrsmodel".to_string())]
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn validate_without_a_file_is_a_usage_error() {
    let home = test_home("validate-none");
    let result = run_rastai(&home, &["validate", "--json"]);
    assert_eq!(result.exit_code, 2, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0002"]);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn version_prints_this_build() {
    let home = test_home("version");
    let text = run_rastai(&home, &["version"]);
    assert_eq!((text.exit_code, text.stdout.trim()), (0, APP_VERSION));
    let as_json = run_rastai(&home, &["version", "--json"]);
    let first: Value = serde_json::from_str(as_json.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first, json!({"version": APP_VERSION}));
    assert_eq!(final_line(&as_json), json!({"exit_code": 0, "diagnostics": []}));
    let _ = fs::remove_dir_all(home);
}

#[test]
fn rebuild_index_makes_an_index_without_starting_a_daemon() {
    let home = test_home("rebuild");
    let result = run_rastai(&home, &["storage", "rebuild-index", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    assert_eq!(final_codes(&result), Vec::<String>::new());
    let report: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(report["runs"], 0);
    assert_eq!(report["old_index_path"], Value::Null);
    assert!(home.join("data").join("rastai.db").exists());
    assert!(daemon_processes(&home).is_empty(), "rebuild-index started a daemon");
    let _ = fs::remove_dir_all(home);
}

/// A daemon holds `daemon.lock` while it runs, so the rebuild refuses (`RRS-S0001`) and changes nothing.
#[cfg(target_os = "linux")]
#[test]
fn rebuild_index_refuses_while_a_daemon_holds_the_lock() {
    let home = test_home("rebuild-locked");
    let _guard = DaemonGuard { home: home.clone() };
    let started = run_rastai(&home, &["system", "status", "--json"]);
    assert_eq!(started.exit_code, 0, "{started:?}");
    let data_files_before = data_file_names(&home);

    let result = run_rastai(&home, &["storage", "rebuild-index", "--json"]);

    assert_eq!(result.exit_code, 1, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-S0001"]);
    assert_eq!(data_file_names(&home), data_files_before);
    assert!(!daemon_processes(&home).is_empty(), "the daemon should still run");
}

/// The names in `<home>/data`, sorted: a refused rebuild adds none (no `rastai.db.rebuilding`, no
/// `rastai.db.broken-*`).
fn data_file_names(home: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(home.join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
