//! The generated JavaScript and Python work in their own languages (layout rust-control-plane.md 4.9).
//!
//! The catalog: for a sample catalog full of awkward text, catalog.js passes the JSDoc check (`tsc --noEmit` in
//! checkJs mode, as `just check-js` runs it) and both modules read back exactly the entries they were generated from.
//!
//! The formats: for the shared header plus sample types that use every field kind, and two sample commands, the
//! generated type modules and command wrappers pass the JSDoc check against the JSON rastai-formats writes (and
//! refuse wrong shapes), run in Node, which would refuse any TypeScript syntax, and the generated Python imports and
//! round-trips that JSON exactly.
//!
//! The real outputs are covered elsewhere: `just check-js` checks packages/catalog and packages/formats, and
//! `just test-python` imports rastai_plugin.catalog and rastai_formats.generated. These tests need the JavaScript
//! tools (`just env-js`) and the cpu environment (`just env-cpu`), which a plain `cargo test` on a CI runner doesn't
//! have, so they are `#[ignore]`d there and run by `just codegen-check`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rastai_codegen::catalog_javascript::render_catalog_javascript;
use rastai_codegen::catalog_python::render_catalog_python;
use rastai_codegen::catalog_source::{CatalogSource, ErrorRecord, TermRecord};
use rastai_codegen::command_source::CommandDescription;
use rastai_codegen::javascript_commands::write_javascript_commands;
use rastai_codegen::javascript_types::write_javascript_types;
use rastai_codegen::literals::severity_word;
use rastai_codegen::python_dataclasses::{snake_case, write_python_dataclasses};
use rastai_codegen::repo_root::find_repo_root;
use rastai_diag::diagnostic::Severity;
use rastai_formats::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of,
};
use rastai_formats::header::{SharedHeader, describe_shared_header, header_fields};
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

// ---- The formats outputs: type modules, command wrappers and dataclasses ----

/// Hand-written files of the formats tree, copied from the repository so the test uses the real ones.
const FORMATS_INDEX_FILE: &str = "packages/formats/src/index.js";
const PYTHON_PACKAGE_INIT_FILE: &str = "python/rastai_formats/rastai_formats/__init__.py";
/// Where the Python import package's parent is, inside a tree laid out like the repository.
const PYTHON_PACKAGE_PARENT: &str = "python/rastai_formats";

/// Stand-ins for @rastai/client's hand-written typedefs (contracts 12.4), which M1-50 writes; the wrappers name them.
const COMMAND_EXECUTOR_STUB: &str = r#"/** @typedef {import("./types.js").ExecuteOutcome} ExecuteOutcome */

/**
 * @callback ExecuteFunction
 * @param {string} commandId
 * @param {object} args
 * @returns {Promise<ExecuteOutcome>}
 */

/**
 * @typedef {object} CommandExecutor
 * @property {ExecuteFunction} execute
 */

export {};
"#;
const CLIENT_TYPES_STUB: &str = r#"/**
 * @typedef {object} ExecuteOutcome
 * @property {boolean} ok
 * @property {any} result
 */

export {};
"#;

/// Uses the formats types and the wrappers the way a package would. Each `@ts-expect-error` line fails tsc unless
/// the line after it is an error, so wrong shapes must be refused. The `__..._JSON__` markers are replaced with the
/// JSON the Rust side writes.
const FORMATS_CONSUMER: &str = r#"import { evalSuiteAddConversation, sampleRun } from "./packages/client/src/generated/commands.js";

/** @typedef {import("@rastai/formats").SharedHeader} SharedHeader */
/** @typedef {import("@rastai/formats").SampleDocument} SampleDocument */
/** @typedef {import("@rastai/formats").SampleRunArgs} SampleRunArgs */
/** @typedef {import("./packages/client/src/command-executor.js").CommandExecutor} CommandExecutor */

/** @type {SharedHeader} */
export const header = __HEADER_JSON__;

/** @type {SampleDocument} */
export const fullDocument = __FULL_DOCUMENT_JSON__;

/** @type {SampleDocument} */
export const minimalDocument = __MINIMAL_DOCUMENT_JSON__;

/** @type {SampleRunArgs} */
export const runArgs = __RUN_ARGS_JSON__;

/** @type {SampleDocument} */
export const wrongMode = {
  ...minimalDocument,
  // @ts-expect-error mode is one of its enum values
  mode: "slow",
};

/** @type {SampleDocument} */
export const unknownKey = {
  ...minimalDocument,
  // @ts-expect-error a key the type doesn't have
  colour: "red",
};

/** @type {SampleDocument} */
export const wrongNestedType = {
  ...minimalDocument,
  // @ts-expect-error root is a SampleNode, not a string
  root: "top",
};

/** @type {CommandExecutor} */
const executor = { execute: async (commandId, args) => ({ ok: true, result: { commandId, args } }) };

export const outcome = sampleRun(executor, runArgs);
export const otherOutcome = evalSuiteAddConversation(executor, runArgs);
// @ts-expect-error the wrapper's arguments are a SampleRunArgs
export const wrongArgs = sampleRun(executor, { document: 1 });
"#;

/// Imports the generated JavaScript in Node and calls each wrapper with a recording executor. Node refuses any
/// TypeScript syntax, so this also proves the files are plain JavaScript.
const FORMATS_NODE_PRINTER: &str = r#"const formats = await import(process.argv[1]);
const commands = await import(process.argv[2]);
const calls = [];
const executor = { execute: async (commandId, args) => { calls.push({ commandId, args }); return { ok: true }; } };
await commands.sampleRun(executor, { from: "cli" });
await commands.evalSuiteAddConversation(executor, {});
console.log(JSON.stringify({ formatsExports: Object.keys(formats), commandExports: Object.keys(commands), calls }));
"#;

/// Reads each case through `<type>_from_json`, checks the result is a frozen dataclass, and prints `<type>_to_json`
/// of it, so the test compares values, not text. Also checks that the reserved word `from` is the field `from_`.
const FORMATS_PYTHON_PRINTER: &str = r#"import dataclasses, json, sys
sys.path.insert(0, sys.argv[1])
from rastai_formats import generated
cases = json.loads(open(sys.argv[2], encoding="utf-8").read())
written = []
for case in cases:
    value = getattr(generated, case["from_json"])(case["value"])
    assert dataclasses.is_dataclass(value), case["from_json"]
    first_field = dataclasses.fields(value)[0].name
    try:
        setattr(value, first_field, None)
        raise SystemExit(f"{type(value).__name__} is not frozen")
    except dataclasses.FrozenInstanceError:
        pass
    written.append(getattr(generated, case["to_json"])(value))
run_args = generated.sample_run_args_from_json(cases[-1]["value"])
assert run_args.from_ == "cli", run_args
assert run_args.document.root.children[0].label == "leaf", run_args
print(json.dumps(written))
"#;

/// The sample document's fields: the header, then one field of each kind.
fn sample_document_fields() -> Vec<FieldDescription> {
    let node = || FieldKind::Type("SampleNode".to_string());
    let mut document_fields = header_fields();
    document_fields.extend([
        field(
            "name",
            FieldKind::String,
            REQUIRED,
            "A \"quoted\" name */ with a comment end.",
        ),
        field("count", FieldKind::Integer, OPTIONAL, "How many."),
        field("ratio", FieldKind::Number, REQUIRED, "A ratio, 0 to 1."),
        field("enabled", FieldKind::Boolean, REQUIRED, ""),
        field("mode", enum_of(&["fast", "exact"]), REQUIRED, "How it runs."),
        field("modes", list_of(enum_of(&["a", "b"])), OPTIONAL, ""),
        field("tags", list_of(FieldKind::String), OPTIONAL, "Line one\nline two."),
        field("limits", FieldKind::Map(Box::new(FieldKind::Integer)), OPTIONAL, ""),
        field("root", node(), REQUIRED, "The root node."),
        field("nodes_by_id", FieldKind::Map(Box::new(node())), OPTIONAL, ""),
        field("grid", list_of(list_of(node())), OPTIONAL, "Rows of nodes."),
        field(
            "fingerprint",
            FieldKind::Proto("rastai.common.v1.HardwareFingerprint".to_string()),
            OPTIONAL,
            "Where it ran.",
        ),
        field("settings", FieldKind::Json, OPTIONAL, "Anything at all."),
    ]);
    document_fields
}

/// The shared header plus sample types that use every field kind: enums, lists, maps, nested and recursive types,
/// a protobuf field, free JSON, a list of lists, a type from another module, a reserved word and awkward doc text.
fn sample_types() -> Vec<TypeDescription> {
    let node = || FieldKind::Type("SampleNode".to_string());
    vec![
        describe_shared_header(),
        sample_type(
            "SampleDocument",
            "rastai.sample/1",
            "A sample document.",
            sample_document_fields(),
        ),
        sample_type(
            "SampleNode",
            "rastai.sample/1",
            "One node of a tree.",
            vec![
                field("label", FieldKind::String, REQUIRED, ""),
                field("children", list_of(node()), OPTIONAL, ""),
                field("weight", FieldKind::Number, OPTIONAL, ""),
            ],
        ),
        sample_type(
            "SampleRunArgs",
            "rastai.command.sample.run/1",
            "Arguments of sample.run.",
            vec![
                field("document", FieldKind::Type("SampleDocument".to_string()), REQUIRED, ""),
                field("from", FieldKind::String, OPTIONAL, "A reserved word in Python."),
                field("notes", list_of(node()), OPTIONAL, ""),
            ],
        ),
        sample_type(
            "SampleRunResult",
            "rastai.command.sample.run/1",
            "",
            vec![field("ok", FieldKind::Boolean, REQUIRED, "")],
        ),
    ]
}

fn sample_type(name: &str, schema_id: &str, doc: &str, fields: Vec<FieldDescription>) -> TypeDescription {
    TypeDescription {
        name: name.to_string(),
        schema_id: schema_id.to_string(),
        doc: doc.to_string(),
        fields,
    }
}

fn sample_commands() -> Vec<CommandDescription> {
    let command = |command_id: &str| CommandDescription {
        command_id: command_id.to_string(),
        title: "Run the sample".to_string(),
        kind: "sync".to_string(),
        stability: "preview".to_string(),
        since_version: "0.1.0".to_string(),
        arguments_type: "SampleRunArgs".to_string(),
        result_type: "SampleRunResult".to_string(),
    };
    vec![command("eval.suite.add_conversation"), command("sample.run")]
}

/// A full header exactly as rastai-formats' serde writes it.
fn header_json() -> Value {
    let header = SharedHeader {
        schema_url: "https://schemas.rastai.dev/rastai.sample/1.json".to_string(),
        document_type: "rastai.sample".to_string(),
        schema_version: 1,
        created_with: "rastai 0.1.0".to_string(),
        id: "sample_01JB3Z8Q4K7F1V0C2D9E6H5M".to_string(),
        content_hash: format!("blake3:{}", "0".repeat(64)),
        derived_from: vec![format!("sample_01JB3Z8Q4K7F1V0C2D9E6H5L@blake3:{}", "1".repeat(64))],
        authors: vec!["Ada".to_string(), "Grace \"Amazing\" Hopper".to_string()],
        license: "Apache-2.0".to_string(),
        source_url: "https://example.org/sample".to_string(),
        requires: [("psychology".to_string(), "^1.0.0".to_string())].into_iter().collect(),
    };
    serde_json::to_value(&header).unwrap()
}

/// A sample document with every field, its header from `header_json`.
fn full_document_json() -> Value {
    let mut document = header_json();
    let fields = json!({
        "name": "a \"quoted\" name ✓", "count": 3, "ratio": 0.25, "enabled": true, "mode": "exact",
        "modes": ["a", "b"], "tags": ["x", "y"], "limits": {"layers": 12},
        "root": {"label": "top", "children": [{"label": "leaf", "weight": 1.5}]},
        "nodes_by_id": {"n1": {"label": "one"}}, "grid": [[{"label": "g00"}], []],
        "fingerprint": {"gpuCount": 1}, "settings": {"anything": [1, null, "x"]},
    });
    if let (Value::Object(members), Value::Object(extra)) = (&mut document, fields) {
        members.extend(extra);
    }
    document
}

/// A sample document with only its required fields.
fn minimal_document_json() -> Value {
    json!({
        "$schema": "https://schemas.rastai.dev/rastai.sample/1.json", "type": "rastai.sample", "schema_version": 1,
        "id": "sample_1", "name": "minimal", "ratio": 1, "enabled": false, "mode": "fast", "root": {"label": "top"},
    })
}

fn run_args_json() -> Value {
    json!({ "document": full_document_json(), "from": "cli", "notes": [{"label": "note"}] })
}

/// Writes the formats outputs for the samples into `folder`, laid out like the repository, with the hand-written
/// files they sit beside.
fn write_formats_tree(folder: &Path) {
    let repo_root = find_repo_root().unwrap();
    write_javascript_types(folder, &sample_types()).unwrap();
    write_javascript_commands(folder, &sample_commands()).unwrap();
    write_python_dataclasses(folder, &sample_types()).unwrap();
    for hand_written in [FORMATS_INDEX_FILE, PYTHON_PACKAGE_INIT_FILE] {
        fs::copy(repo_root.join(hand_written), folder.join(hand_written)).unwrap();
    }
    fs::write(
        folder.join("packages/client/src/command-executor.js"),
        COMMAND_EXECUTOR_STUB,
    )
    .unwrap();
    fs::write(folder.join("packages/client/src/types.js"), CLIENT_TYPES_STUB).unwrap();
    fs::write(folder.join("package.json"), r#"{ "type": "module" }"#).unwrap();
}

fn formats_consumer() -> String {
    FORMATS_CONSUMER
        .replace("__HEADER_JSON__", &header_json().to_string())
        .replace("__FULL_DOCUMENT_JSON__", &full_document_json().to_string())
        .replace("__MINIMAL_DOCUMENT_JSON__", &minimal_document_json().to_string())
        .replace("__RUN_ARGS_JSON__", &run_args_json().to_string())
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_types_and_wrappers_pass_the_jsdoc_check_against_the_shared_header() {
    let repo_root = find_repo_root().unwrap();
    let folder = scratch_folder("formats-jsdoc");
    write_formats_tree(&folder);
    fs::write(folder.join("use-formats.js"), formats_consumer()).unwrap();
    let base_settings = repo_root.join("jsconfig.base.json");
    let settings = json!({
        "extends": base_settings.to_str().unwrap(),
        "compilerOptions": { "paths": { "@rastai/formats": ["./packages/formats/src/index.js"] } },
        "include": ["use-formats.js", "packages"],
    });
    let settings_path = folder.join("jsconfig.json");
    fs::write(&settings_path, settings.to_string()).unwrap();
    run(
        &repo_root.join(TYPESCRIPT_CHECKER),
        &["--noEmit", "-p", settings_path.to_str().unwrap()],
    );
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_javascript_runs_in_node_and_the_wrappers_call_the_executor() {
    let folder = scratch_folder("formats-node");
    write_formats_tree(&folder);
    let formats_url = format!("file://{}", folder.join(FORMATS_INDEX_FILE).display());
    let commands_path = folder.join("packages/client/src/generated/commands.js");
    let commands_url = format!("file://{}", commands_path.display());
    let printed = run(
        Path::new("node"),
        &[
            "--input-type=module",
            "--eval",
            FORMATS_NODE_PRINTER,
            &formats_url,
            &commands_url,
        ],
    );
    let expected = json!({
        "formatsExports": [],
        "commandExports": ["evalSuiteAddConversation", "sampleRun"],
        "calls": [
            {"commandId": "sample.run", "args": {"from": "cli"}},
            {"commandId": "eval.suite.add_conversation", "args": {}},
        ],
    });
    assert_eq!(parse_printed_json(&printed), expected);
    fs::remove_dir_all(&folder).unwrap();
}

#[test]
#[ignore = "needs `just env-js` and `just env-cpu`; `just codegen-check` runs it"]
fn generated_python_round_trips_the_shared_header_and_every_field_kind() {
    let folder = scratch_folder("formats-python");
    write_formats_tree(&folder);
    let cases = [
        ("SharedHeader", header_json()),
        ("SampleDocument", full_document_json()),
        ("SampleDocument", minimal_document_json()),
        ("SampleRunArgs", run_args_json()),
    ];
    let case_list: Vec<Value> = cases
        .iter()
        .map(|(type_name, value)| {
            let function_base = snake_case(type_name);
            json!({ "from_json": format!("{function_base}_from_json"), "to_json": format!("{function_base}_to_json"), "value": value })
        })
        .collect();
    let cases_path = folder.join("cases.json");
    fs::write(&cases_path, Value::Array(case_list).to_string()).unwrap();
    let python = find_repo_root().unwrap().join(CPU_PYTHON);
    let package_parent = folder.join(PYTHON_PACKAGE_PARENT);
    let printed = run(
        &python,
        &[
            "-I",
            "-c",
            FORMATS_PYTHON_PRINTER,
            package_parent.to_str().unwrap(),
            cases_path.to_str().unwrap(),
        ],
    );
    let expected: Vec<Value> = cases.into_iter().map(|(_, value)| value).collect();
    assert_eq!(parse_printed_json(&printed), Value::Array(expected));
    fs::remove_dir_all(&folder).unwrap();
}
