//! Shape inference for each op equals its golden file, `fixtures/ops/<op>.json` (layout 4.11, contracts 5.3 and
//! 2.4 rule 10). The same files hold each case's parameter count and FLOPs, which the Python compiler and Rust live
//! stats match; here every op with shapes must have a file, every file must name such an op, and every file must
//! state the same FLOP convention. fixtures/ops/README.md describes the format.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rastai_formats::documents::model_ops::{OPS_WITH_SHAPES, op_attributes_from_value};
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::meaning::model_shapes::infer_op_output_shapes;
use serde_json::Value;

const OPS_FOLDER: &str = "../../fixtures/ops";
const GOLDEN_EXTENSION: &str = "json";

/// The golden files, sorted.
fn golden_files() -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join(OPS_FOLDER);
    let mut files: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == GOLDEN_EXTENSION))
        .collect();
    files.sort();
    files
}

/// A golden file through the strict reader, so a duplicate key or a NaN in a golden can't slip through.
fn read_golden(path: &Path) -> Value {
    read_json(&fs::read(path).unwrap(), &ImportLimits::default())
        .unwrap()
        .value
}

fn shapes_of(value: &Value) -> Vec<Vec<u64>> {
    serde_json::from_value(value.clone()).unwrap()
}

/// Runs every case of one file and returns a failure message per case that differs.
fn check_golden(path: &Path, golden: &Value) -> Vec<String> {
    let op = golden["op"].as_str().unwrap();
    let mut failures = Vec::new();
    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let attributes: BTreeMap<String, Value> = serde_json::from_value(case["attributes"].clone()).unwrap();
        let typed = op_attributes_from_value(op, &attributes).unwrap().unwrap();
        let input_shapes = shapes_of(&case["input_shapes"]);
        let expected = shapes_of(&case["output_shapes"]);
        match infer_op_output_shapes(&typed, &input_shapes) {
            Ok(output_shapes) if output_shapes == expected => {}
            other => failures.push(format!(
                "{} {name}: expected {expected:?}, got {other:?}",
                path.display()
            )),
        }
    }
    failures
}

#[test]
fn shape_inference_for_each_op_equals_its_golden_file() {
    let mut failures = Vec::new();
    let mut case_count = 0;
    for path in golden_files() {
        let golden = read_golden(&path);
        case_count += golden["cases"].as_array().unwrap().len();
        failures.extend(check_golden(&path, &golden));
    }
    assert!(case_count > 0, "the goldens hold no case: the test would check nothing");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_op_with_shapes_has_one_golden_file_named_for_it() {
    let mut named_ops = Vec::new();
    for path in golden_files() {
        let golden = read_golden(&path);
        let op = golden["op"].as_str().unwrap().to_string();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(stem, op, "{} names another op", path.display());
        named_ops.push(op);
    }
    let mut ops_with_shapes: Vec<String> = OPS_WITH_SHAPES.iter().map(|op| op.to_string()).collect();
    ops_with_shapes.sort();
    assert_eq!(named_ops, ops_with_shapes);
}

#[test]
fn every_golden_states_the_same_flop_convention_and_every_number() {
    let mut conventions = Vec::new();
    for path in golden_files() {
        let golden = read_golden(&path);
        let convention = golden["flop_convention"].as_str().unwrap().to_string();
        assert!(convention.contains("FlopCounterMode"), "{}", path.display());
        conventions.push(convention);
        for case in golden["cases"].as_array().unwrap() {
            let numbers_present = case["parameter_count"].is_u64() && case["flops"].is_u64();
            assert!(numbers_present, "{} {}", path.display(), case["name"]);
        }
    }
    conventions.dedup();
    assert_eq!(conventions.len(), 1, "the files state different conventions");
}
