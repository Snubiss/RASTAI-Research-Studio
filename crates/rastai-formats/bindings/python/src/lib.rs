//! `rastai_formats._native`: rastai-formats for Python (layout: rust-control-plane.md section 4.7). The only file
//! that imports pyo3 (contracts 2.5). python/rastai_formats wraps it; workers never call it directly.
//!
//! Every function takes and returns `bytes` and `str`, so no Rust type crosses into Python. A failure raises
//! `FormatsValidationError`, whose one argument is the diagnostics as a JSON array (rastai-diag's `render_json` for
//! each). Every rule here is rastai-formats' own: this file only reads the arguments, calls it and shapes the answer.
//!
//! The functions release the GIL while rastai-formats works, because a document can be large (up to the 64 MiB of
//! `ImportLimits`) and other Python threads, such as the worker's gRPC handlers, must keep running meanwhile.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use rastai_diag::diagnostic::{Diagnostic, diagnostic, diagnostic_at, has_errors};
use rastai_diag::path::{DiagnosticPath, format_path};
use rastai_diag::render_json::render_json;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::model::MODEL_TYPE_NAME;
use rastai_formats::documents::{Document, document_header};
use rastai_formats::error::FormatsError;
use rastai_formats::json::canonical::canonical_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{ReadOutcome, document_content_hash, read_document, write_document};
use serde_json::{Map, Value};

/// The name `write_document` and `content_hash` read their text under. It is no real file: the document's `type`
/// field says what it is. It shows in the paths of their diagnostics (`document.json:3:5#/step`).
const DOCUMENT_TEXT_NAME: &str = "document.json";
/// The name `load_model_spec` reads its bytes under: the ModelSpec extension, so the type check knows what to expect.
const MODEL_FILE_NAME: &str = "model.rrsmodel";
const TYPE_POINTER: &str = "/type";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";

// The keys of `read_document`'s answer, the same JSON as the WASM build's readDocument (layout 4.6).
const DOCUMENT_JSON_KEY: &str = "document_json";
const DIAGNOSTICS_KEY: &str = "diagnostics";
const READ_ONLY_KEY: &str = "read_only";
const MIGRATED_FROM_VERSION_KEY: &str = "migrated_from_version";

create_exception!(
    rastai_formats,
    FormatsValidationError,
    PyException,
    "A document failed validation or couldn't be written. Its one argument is the diagnostics, a JSON array."
);

/// Reads a document's bytes: `{"document_json", "diagnostics", "read_only", "migrated_from_version"}` as JSON text.
/// `document_json` is the typed document as JSON text, empty when layers 1 or 2 refused it. Nothing raises for a bad
/// file: the diagnostics are the answer. `file_name` names the type by its extension and starts every path.
#[pyfunction(name = "read_document")]
fn read_document_py(py: Python<'_>, file_name: &str, data: &[u8]) -> PyResult<String> {
    let answer = py.detach(|| outcome_json(&read_bytes(file_name, data)));
    answer.map_err(|diagnostics| validation_error(&diagnostics))
}

/// The ModelSpec in `json_bytes`, validated (layers 1 to 3 with no error), migrated to the current version and
/// written as canonical JSON (sorted keys, no spaces): the one form the Python compiler parses (spec §16).
#[pyfunction(name = "load_model_spec")]
fn load_model_spec_py(py: Python<'_>, json_bytes: &[u8]) -> PyResult<String> {
    let answer = py.detach(|| model_spec_json(json_bytes));
    answer.map_err(|diagnostics| validation_error(&diagnostics))
}

/// The bytes of a document's file: validated, its `content_hash` filled in, in the stable pretty form. Workers write
/// run.json and resume.json with it.
#[pyfunction(name = "write_document")]
fn write_document_py<'py>(py: Python<'py>, document_json: &str) -> PyResult<Bound<'py, PyBytes>> {
    let answer = py.detach(|| written_bytes(document_json));
    match answer {
        Ok(bytes) => Ok(PyBytes::new(py, &bytes)),
        Err(diagnostics) => Err(validation_error(&diagnostics)),
    }
}

/// A validated document's content hash, `blake3:<64 hex>`: the one `write_document` writes into it (contracts 5.1).
#[pyfunction(name = "content_hash")]
fn content_hash_py(py: Python<'_>, document_json: &str) -> PyResult<String> {
    let answer = py.detach(|| hash_of_text(document_json));
    answer.map_err(|diagnostics| validation_error(&diagnostics))
}

/// The BLAKE3 hash of some bytes, `blake3:<64 hex>` (contracts 8.6).
#[pyfunction(name = "hash_bytes")]
fn hash_bytes_py(py: Python<'_>, data: &[u8]) -> String {
    py.detach(|| rastai_formats::hashing::hash_bytes(data))
}

#[pymodule]
#[pyo3(name = "_native")]
fn native_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(read_document_py, module)?)?;
    module.add_function(wrap_pyfunction!(load_model_spec_py, module)?)?;
    module.add_function(wrap_pyfunction!(write_document_py, module)?)?;
    module.add_function(wrap_pyfunction!(content_hash_py, module)?)?;
    module.add_function(wrap_pyfunction!(hash_bytes_py, module)?)?;
    let exception_type = module.py().get_type::<FormatsValidationError>();
    module.add("FormatsValidationError", exception_type)?;
    Ok(())
}

fn read_bytes(file_name: &str, data: &[u8]) -> ReadOutcome {
    read_document(file_name, &SliceSource { bytes: data }, &ImportLimits::default())
}

fn outcome_json(outcome: &ReadOutcome) -> Result<String, Vec<Diagnostic>> {
    let document_json = match &outcome.document {
        Some(document) => document_text(document)?,
        None => String::new(),
    };
    let mut answer = Map::new();
    answer.insert(DOCUMENT_JSON_KEY.to_string(), Value::String(document_json));
    answer.insert(DIAGNOSTICS_KEY.to_string(), diagnostics_value(&outcome.diagnostics));
    answer.insert(READ_ONLY_KEY.to_string(), Value::Bool(outcome.read_only));
    let migrated_from_version = Value::from(outcome.migrated_from_version);
    answer.insert(MIGRATED_FROM_VERSION_KEY.to_string(), migrated_from_version);
    Ok(Value::Object(answer).to_string())
}

fn model_spec_json(json_bytes: &[u8]) -> Result<String, Vec<Diagnostic>> {
    let document = sound_document(MODEL_FILE_NAME, json_bytes)?;
    let type_name = &document_header(&document).document_type;
    if type_name != MODEL_TYPE_NAME {
        return Err(vec![not_a_model_spec(type_name)]);
    }
    Ok(canonical_json(&document_value(&document)?))
}

fn written_bytes(document_json: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let document = sound_document(DOCUMENT_TEXT_NAME, document_json.as_bytes())?;
    write_document(&document).map_err(|error| vec![formats_error_diagnostic(&error)])
}

fn hash_of_text(document_json: &str) -> Result<String, Vec<Diagnostic>> {
    let document = sound_document(DOCUMENT_TEXT_NAME, document_json.as_bytes())?;
    document_content_hash(&document).map_err(|error| vec![formats_error_diagnostic(&error)])
}

/// The typed document, when it read with no error and isn't from a newer version. A newer file opens read-only:
/// writing or hashing it here would drop the fields this version doesn't know, so it is refused with its RRS-E4001.
fn sound_document(file_name: &str, data: &[u8]) -> Result<Document, Vec<Diagnostic>> {
    let outcome = read_bytes(file_name, data);
    if outcome.read_only || has_errors(&outcome.diagnostics) {
        return Err(outcome.diagnostics);
    }
    outcome.document.ok_or(outcome.diagnostics)
}

/// RRS-E1004 at /type: a valid document of another type where a ModelSpec belongs.
fn not_a_model_spec(type_name: &str) -> Diagnostic {
    let path = DiagnosticPath {
        file: MODEL_FILE_NAME.to_string(),
        json_pointer: TYPE_POINTER.to_string(),
        ..DiagnosticPath::default()
    };
    let values = [("found", type_name), ("expected", MODEL_TYPE_NAME)];
    diagnostic_at(CODE_VALUE_NOT_ALLOWED, &values, &format_path(&path))
}

fn document_value(document: &Document) -> Result<Value, Vec<Diagnostic>> {
    serde_json::to_value(document).map_err(|error| vec![not_serializable(&error.to_string())])
}

fn document_text(document: &Document) -> Result<String, Vec<Diagnostic>> {
    serde_json::to_string(document).map_err(|error| vec![not_serializable(&error.to_string())])
}

fn not_serializable(reason: &str) -> Diagnostic {
    let error = FormatsError::NotSerializable {
        reason: reason.to_string(),
    };
    formats_error_diagnostic(&error)
}

/// Writing and hashing fail only when serde can't write a value (RRS-E1006, a bug rather than a bad file), whose
/// catalog message names the reason.
fn formats_error_diagnostic(error: &FormatsError) -> Diagnostic {
    let reason = match error {
        FormatsError::NotSerializable { reason } => reason.clone(),
        other => format!("{other:?}"),
    };
    diagnostic(error.code(), &[("reason", &reason)])
}

fn diagnostics_value(diagnostics: &[Diagnostic]) -> Value {
    Value::Array(diagnostics.iter().map(render_json).collect())
}

fn validation_error(diagnostics: &[Diagnostic]) -> PyErr {
    FormatsValidationError::new_err(diagnostics_value(diagnostics).to_string())
}
