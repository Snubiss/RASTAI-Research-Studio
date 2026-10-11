//! The generated JavaScript and Python work in their own languages (layout rust-control-plane.md 4.9): for a sample
//! catalog full of awkward text, catalog.js passes the JSDoc check (`tsc --noEmit` in checkJs mode, as `just check-js`
//! runs it) and both modules read back exactly the entries they were generated from.
//!
//! The real catalog's outputs are covered elsewhere: `just check-js` checks packages/catalog and `just test-python`
//! imports rastai_plugin.catalog. These tests need the JavaScript tools (`just env-js`) and the cpu environment
//! (`just env-cpu`), which a plain `cargo test` on a CI runner doesn't have, so they are `#[ignore]`d there and run
//! by `just codegen-check`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rastai_codegen::catalog_javascript::render_catalog_javascript;
use rastai_codegen::catalog_python::render_catalog_python;
use rastai_codegen::catalog_source::{CatalogSource, ErrorRecord, TermRecord};
use rastai_codegen::literals::severity_word;
use rastai_codegen::repo_root::find_repo_root;
use rastai_diag::diagnostic::Severity;
use serde_json::{Value, json};

const TYPESCRIPT_CHECKER: &str = "node_modules/.bin/tsc";
const CPU_PYTHON: &str = ".dev/envs/cpu/bin/python";
const IGNORE_REASON: &str = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it";

/// Uses the generated module the way a package would: imports both arrays and names both typedefs. tsc fails if a
/// typedef is missing or an entry doesn't match it.
const JAVASCRIPT_CONSUMER: &str = r#"import { CATALOG, TERMS } from "./catalog.js";

/** @typedef {import("./catalog.js").CatalogEntry} CatalogEntry */
/** @typedef {import("./catalog.js").TermEntry} TermEntry */

/**
 * @param {string} code
 * @returns {CatalogEntry | undefined}
 */
export function findEntry(code) {
  return CATALOG.find((entry) => entry.code === code);
}

/** @type {"error" | "warning" | "info" | undefined} */
export const firstSeverity = CATALOG[0]?.severity;

/** @type {TermEntry | undefined} */
export const firstTerm = TERMS[0];
"#;

/// Prints the module's arrays as JSON, so the test compares values, not text.
const NODE_PRINTER: &str = r#"const catalogModule = await import(process.argv[1]);
console.log(JSON.stringify({ errors: catalogModule.CATALOG, terms: catalogModule.TERMS }));
"#;

const PYTHON_PRINTER: &str = r#"import dataclasses, json, runpy, sys
module = runpy.run_path(sys.argv[1])
errors = [dataclasses.asdict(entry) for entry in module["CATALOG"]]
terms = [dataclasses.asdict(entry) for entry in module["TERMS"]]
for entry in errors:
    assert module[entry["code"].replace("-", "_")] == entry["code"]
print(json.dumps({"errors": errors, "terms": terms}))
"#;

/// Text that breaks naive string writing: quotes of each kind, backslashes, line breaks, tabs, template and format
/// syntax, a closing script tag, a control character and non-ASCII letters.
fn awkward_catalog() -> CatalogSource {
    CatalogSource {
        errors: vec![
            ErrorRecord {
                code: "RRS-E0003".to_string(),
                severity: Severity::Error,
                title: "Duplicate key in a JSON object".to_string(),
                message: "The key \"{key}\" appears twice at {path}; 'single' `back` ${not_a_template} %s".to_string(),
                basic_text: "Ein Name steht zweimal — é ✓ 名前".to_string(),
                fix_text: "Remove one.\nThen save.\tC:\\path\\file </script>".to_string(),
                fix_command_id: String::new(),
                docs_anchor: "json-duplicate-key".to_string(),
                retired: false,
                source_file: "catalog/errors/RRS-E.json".to_string(),
            },
            ErrorRecord {
                code: "RRS-S3001".to_string(),
                severity: Severity::Warning,
                title: "Not enough free disk space".to_string(),
                message: "bell \u{7} and \u{2028} line separator".to_string(),
                basic_text: "Your disk is almost full.".to_string(),
                fix_text: "Free up space.".to_string(),
                fix_command_id: "storage.gc".to_string(),
                docs_anchor: "storage-disk-full".to_string(),
                retired: true,
                source_file: "catalog/errors/RRS-S.json".to_string(),
            },
        ],
        terms: vec![TermRecord {
            term_id: "logit_lens".to_string(),
            title: "Logit lens".to_string(),
            basic_text: "What each layer \"would\" predict.".to_string(),
            advanced_text: "The residual stream, unembedded:\n\\(W_U h\\).".to_string(),
            docs_anchor: "logit-lens".to_string(),
            source_file: "catalog/terms/terms.json".to_string(),
        }],
    }
}

/// The catalog as the JSON both printers should print.
fn expected_json(catalog: &CatalogSource) -> Value {
    let errors: Vec<Value> = catalog
        .errors
        .iter()
        .map(|record| {
            json!({
                "code": record.code, "severity": severity_word(record.severity), "title": record.title,
                "message": record.message, "basic_text": record.basic_text, "fix_text": record.fix_text,
                "fix_command_id": record.fix_command_id, "docs_anchor": record.docs_anchor, "retired": record.retired,
            })
        })
        .collect();
    let terms: Vec<Value> = catalog
        .terms
        .iter()
        .map(|record| {
            json!({
                "term_id": record.term_id, "title": record.title, "basic_text": record.basic_text,
                "advanced_text": record.advanced_text, "docs_anchor": record.docs_anchor,
            })
        })
        .collect();
    json!({ "errors": errors, "terms": terms })
}

fn scratch_folder(test_name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rastai-codegen-outputs-{}-{test_name}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

/// Runs a program and returns its stdout; a failure fails the test with the program's output.
fn run(program: &Path, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(find_repo_root().unwrap())
        .output()
        .unwrap_or_else(|error| panic!("{} doesn't run ({error}); {IGNORE_REASON}", program.display()));
    assert!(
        output.status.success(),
        "{} {} failed:\n{}{}",
        program.display(),
        arguments.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn parse_printed_json(printed: &str) -> Value {
    serde_json::from_str(printed.trim()).unwrap()
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_javascript_passes_the_jsdoc_check() {
    let repo_root = find_repo_root().unwrap();
    let folder = scratch_folder("jsdoc");
    fs::write(folder.join("catalog.js"), render_catalog_javascript(&awkward_catalog())).unwrap();
    fs::write(folder.join("use-catalog.js"), JAVASCRIPT_CONSUMER).unwrap();
    let base_settings = repo_root.join("jsconfig.base.json");
    let settings = json!({ "extends": base_settings.to_str().unwrap(), "include": ["*.js"] });
    fs::write(folder.join("jsconfig.json"), settings.to_string()).unwrap();
    let settings_path = folder.join("jsconfig.json");
    run(
        &repo_root.join(TYPESCRIPT_CHECKER),
        &["--noEmit", "-p", settings_path.to_str().unwrap()],
    );
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_javascript_reads_back_the_catalog() {
    let catalog = awkward_catalog();
    let folder = scratch_folder("node");
    let module_path = folder.join("catalog.mjs");
    fs::write(&module_path, render_catalog_javascript(&catalog)).unwrap();
    let module_url = format!("file://{}", module_path.display());
    let printed = run(
        Path::new("node"),
        &["--input-type=module", "--eval", NODE_PRINTER, &module_url],
    );
    assert_eq!(parse_printed_json(&printed), expected_json(&catalog));
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_python_reads_back_the_catalog() {
    let catalog = awkward_catalog();
    let folder = scratch_folder("python");
    let module_path = folder.join("generated.py");
    fs::write(&module_path, render_catalog_python(&catalog)).unwrap();
    let python = find_repo_root().unwrap().join(CPU_PYTHON);
    let printed = run(&python, &["-I", "-c", PYTHON_PRINTER, module_path.to_str().unwrap()]);
    assert_eq!(parse_printed_json(&printed), expected_json(&catalog));
    fs::remove_dir_all(&folder).unwrap();
}
