//! The Python types output: `python/rastai_formats/rastai_formats/generated/`, read as `rastai_formats.generated`
//! (layout rust-control-plane.md 4.9, python.md 1.6). One module per schema id (type_modules.rs), and in it, per
//! type: a frozen dataclass (contracts 12.3: data is a dataclass), `<type>_from_json` and `<type>_to_json`.
//! `__init__.py` re-exports them all.
//!
//! The dataclasses are keyword-only, so fields keep the description's order (the order files are written in) with
//! optional fields, which default to None, between required ones. An optional field that is None is left out of
//! `to_json`, so a file read and written back keeps exactly its keys. A protobuf field is a plain dict whose comment
//! names its message, so `rastai_formats` doesn't depend on `rastai_proto` (contracts 4). Neither function
//! validates: documents are validated by rastai-formats itself, through the wheel (M1-19).

use std::path::Path;

use rastai_formats::describe::types::{FieldDescription, FieldKind, TypeDescription};

use crate::error::{CodegenError, codegen_error};
use crate::literals::{FORMATS_SOURCE_NOTE, GENERATED_NOTICE, json_string_literal};
use crate::type_modules::{
    DESCRIPTIONS_SOURCE_FILE, TypeModule, group_types_into_modules, holds_a_described_type, module_of_type,
    referenced_type_names,
};
use crate::write_if_changed::{remove_unwritten_files, write_if_changed};

const GENERATED_FOLDER: &str = "python/rastai_formats/rastai_formats/generated";
const IMPORT_PACKAGE: &str = "rastai_formats.generated";
const INIT_FILE: &str = "__init__.py";
const INDENT: &str = "    ";
const DOUBLE_INDENT: &str = "        ";
const FROM_JSON_SUFFIX: &str = "from_json";
const TO_JSON_SUFFIX: &str = "to_json";
/// Python's reserved words (3.12). A JSON key that is one gets a trailing underscore: `from` is the field `from_`.
/// Soft keywords such as `type` and `match` are fine as field names.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

const MODULE_DOCSTRING_TAIL: &str = "
from_json reads a dict as json.loads returns it; to_json gives one back that json.dumps writes with the same keys.
Neither validates: rastai-formats validates documents (through the wheel) before they reach these classes.
\"\"\"

from __future__ import annotations

from dataclasses import dataclass
";

// A `\` at the end of a line drops the line break, so the string starts with the docstring's opening quotes.
const INIT_DOCSTRING: &str = "\
\"\"\"Every type rastai-formats describes, as frozen dataclasses with from_json and to_json functions (contracts 5.4).

Import them from here: `from rastai_formats.generated import SharedHeader, shared_header_from_json`.
\"\"\"
";

/// Writes every module and `__init__.py` under `output_root`, then deletes the modules it no longer writes (the
/// folder is this generator's alone; `__pycache__/` is left alone). Returns how many files changed or were deleted.
pub fn write_python_dataclasses(output_root: &Path, all_types: &[TypeDescription]) -> Result<usize, CodegenError> {
    let modules = group_types_into_modules(all_types)?;
    let folder = output_root.join(GENERATED_FOLDER);
    let mut changed_count = 0;
    let mut written_paths = Vec::new();
    for module in &modules {
        let contents = render_python_module(module, all_types)?;
        let path = folder.join(format!("{}.py", module.name));
        if write_if_changed(&path, &contents)? {
            changed_count += 1;
        }
        written_paths.push(path);
    }
    let init_path = folder.join(INIT_FILE);
    if write_if_changed(&init_path, &render_python_init(&modules))? {
        changed_count += 1;
    }
    written_paths.push(init_path);
    changed_count += remove_unwritten_files(&folder, &written_paths)?;
    Ok(changed_count)
}

/// `__init__.py`: one import line per module and `__all__` naming everything it imports.
pub fn render_python_init(modules: &[TypeModule]) -> String {
    let mut text = notice_lines();
    text.push_str(INIT_DOCSTRING);
    let mut exported_names = Vec::new();
    if !modules.is_empty() {
        text.push('\n');
    }
    for module in modules {
        let names: Vec<String> = module
            .types
            .iter()
            .flat_map(|description| exported_names_of(&description.name))
            .collect();
        text.push_str(&format!(
            "from {IMPORT_PACKAGE}.{} import {}\n",
            module.name,
            names.join(", ")
        ));
        exported_names.extend(names);
    }
    if exported_names.is_empty() {
        text.push_str("\n__all__: list[str] = []\n");
        return text;
    }
    text.push_str("\n__all__ = [\n");
    for name in &exported_names {
        text.push_str(&format!("{INDENT}{},\n", json_string_literal(name)));
    }
    text.push_str("]\n");
    text
}

/// The class and its two functions: `SharedHeader`, `shared_header_from_json`, `shared_header_to_json`.
fn exported_names_of(type_name: &str) -> Vec<String> {
    let function_base = snake_case(type_name);
    vec![
        type_name.to_string(),
        format!("{function_base}_{FROM_JSON_SUFFIX}"),
        format!("{function_base}_{TO_JSON_SUFFIX}"),
    ]
}

/// One module: notice, docstring, imports, then each type's dataclass and its two functions.
pub fn render_python_module(module: &TypeModule, all_types: &[TypeDescription]) -> Result<String, CodegenError> {
    let mut text = notice_lines();
    text.push_str(&module_docstring_head(module));
    text.push_str(MODULE_DOCSTRING_TAIL);
    text.push_str(&typing_import_line(module));
    text.push_str(&other_module_imports(module, all_types)?);
    for description in &module.types {
        let fields = python_fields(description)?;
        text.push_str("\n\n");
        text.push_str(&render_dataclass(description, &fields));
        text.push_str("\n\n");
        text.push_str(&render_from_json(description, &fields));
        text.push_str("\n\n");
        text.push_str(&render_to_json(description, &fields));
    }
    Ok(text)
}

fn notice_lines() -> String {
    format!("# {GENERATED_NOTICE}\n# {FORMATS_SOURCE_NOTE}\n\n")
}

fn module_docstring_head(module: &TypeModule) -> String {
    if module.schema_id.is_empty() {
        return "\"\"\"The types no one document owns (the shared header), as frozen dataclasses (contracts 5.4).\n"
            .to_string();
    }
    format!(
        "\"\"\"The types of {} as frozen dataclasses (contracts 5.4).\n",
        module.schema_id
    )
}

/// `from typing import Any`, with `Literal` when a field is an enum.
fn typing_import_line(module: &TypeModule) -> String {
    let uses_literal = module
        .types
        .iter()
        .flat_map(|description| description.fields.iter())
        .any(|field| holds_an_enum(&field.kind));
    if uses_literal {
        "from typing import Any, Literal\n".to_string()
    } else {
        "from typing import Any\n".to_string()
    }
}

fn holds_an_enum(kind: &FieldKind) -> bool {
    match kind {
        FieldKind::Enum(_) => true,
        FieldKind::List(inner) | FieldKind::Map(inner) => holds_an_enum(inner),
        _ => false,
    }
}

/// One `from rastai_formats.generated.<module> import ...` line per other module whose types this one names.
fn other_module_imports(module: &TypeModule, all_types: &[TypeDescription]) -> Result<String, CodegenError> {
    let mut imports_by_module: Vec<(String, Vec<String>)> = Vec::new();
    for type_name in referenced_type_names(module) {
        let type_module = module_of_type(&type_name, all_types)?;
        if type_module == module.name {
            continue;
        }
        let names = exported_names_of(&type_name);
        match imports_by_module.iter_mut().find(|(name, _)| *name == type_module) {
            Some((_, imported)) => imported.extend(names),
            None => imports_by_module.push((type_module, names)),
        }
    }
    imports_by_module.sort_by(|left, right| left.0.cmp(&right.0));
    let mut text = String::new();
    if !imports_by_module.is_empty() {
        text.push('\n');
    }
    for (module_name, names) in imports_by_module {
        text.push_str(&format!(
            "from {IMPORT_PACKAGE}.{module_name} import {}\n",
            names.join(", ")
        ));
    }
    Ok(text)
}

/// A field as Python sees it: its attribute name beside its JSON key.
struct PythonField<'a> {
    attribute: String,
    description: &'a FieldDescription,
}

fn python_fields(description: &TypeDescription) -> Result<Vec<PythonField<'_>>, CodegenError> {
    let mut fields: Vec<PythonField> = Vec::new();
    for field in &description.fields {
        let attribute = python_field_name(&field.name).ok_or_else(|| {
            field_error(
                description,
                field,
                "can't be a Python attribute name; JSON keys must be snake_case words",
            )
        })?;
        // `Literal[]` is a Python syntax error, and an enum no value satisfies is a mistake in the description.
        if holds_an_empty_enum(&field.kind) {
            return Err(field_error(description, field, "is an enum with no values"));
        }
        if fields.iter().any(|existing| existing.attribute == attribute) {
            return Err(field_error(
                description,
                field,
                &format!("gives the attribute `{attribute}` twice"),
            ));
        }
        fields.push(PythonField {
            attribute,
            description: field,
        });
    }
    Ok(fields)
}

fn holds_an_empty_enum(kind: &FieldKind) -> bool {
    match kind {
        FieldKind::Enum(values) => values.is_empty(),
        FieldKind::List(inner) | FieldKind::Map(inner) => holds_an_empty_enum(inner),
        _ => false,
    }
}

fn field_error(description: &TypeDescription, field: &FieldDescription, problem: &str) -> CodegenError {
    codegen_error(
        Path::new(DESCRIPTIONS_SOURCE_FILE),
        format!("{}.{} {problem}", description.name, field.name),
    )
}

/// The attribute name for a JSON key: `$schema` is `schema` (a `$` can't be in a Python name), a reserved word
/// gets a trailing underscore (`from_`), and anything else that isn't an ASCII identifier is refused (None).
pub fn python_field_name(json_key: &str) -> Option<String> {
    let name = json_key.replace('$', "");
    let mut characters = name.chars();
    let first = characters.next()?;
    let is_start = first.is_ascii_alphabetic() || first == '_';
    if !is_start || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_') {
        return None;
    }
    if PYTHON_KEYWORDS.contains(&name.as_str()) {
        return Some(format!("{name}_"));
    }
    Some(name)
}

/// The snake_case of a type name, for its functions: `SharedHeader` is `shared_header`, `HTTPServer` is
/// `http_server`. An underscore goes before a capital that starts a new word: after a lowercase letter or digit, or
/// after a capital when a lowercase letter follows (the end of an acronym).
pub fn snake_case(type_name: &str) -> String {
    let characters: Vec<char> = type_name.chars().collect();
    let mut name = String::new();
    for (index, character) in characters.iter().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            let previous = characters[index - 1];
            let next_is_lowercase = characters.get(index + 1).is_some_and(|next| next.is_ascii_lowercase());
            let starts_word = previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_is_lowercase);
            if starts_word {
                name.push('_');
            }
        }
        name.push(character.to_ascii_lowercase());
    }
    name
}

/// The type annotation of a field kind.
pub fn python_type(kind: &FieldKind) -> String {
    match kind {
        FieldKind::String => "str".to_string(),
        FieldKind::Integer => "int".to_string(),
        FieldKind::Number => "float".to_string(),
        FieldKind::Boolean => "bool".to_string(),
        FieldKind::Json => "Any".to_string(),
        FieldKind::Proto(_) => "dict[str, Any]".to_string(),
        FieldKind::Enum(values) => {
            let literals: Vec<String> = values.iter().map(|value| json_string_literal(value)).collect();
            format!("Literal[{}]", literals.join(", "))
        }
        FieldKind::List(element_kind) => format!("list[{}]", python_type(element_kind)),
        FieldKind::Map(member_kind) => format!("dict[str, {}]", python_type(member_kind)),
        FieldKind::Type(type_name) => type_name.clone(),
    }
}

fn render_dataclass(description: &TypeDescription, fields: &[PythonField]) -> String {
    let mut text = format!("@dataclass(frozen=True, kw_only=True)\nclass {}:\n", description.name);
    if !description.doc.is_empty() {
        text.push_str(&format!("{INDENT}{}\n", json_string_literal(&description.doc)));
    }
    if fields.is_empty() && description.doc.is_empty() {
        text.push_str(&format!("{INDENT}pass\n"));
    }
    if !fields.is_empty() && !description.doc.is_empty() {
        text.push('\n');
    }
    for field in fields {
        let comment = field_comment(field);
        if !comment.is_empty() {
            text.push_str(&format!("{INDENT}# {comment}\n"));
        }
        let annotation = python_type(&field.description.kind);
        if field.description.required {
            text.push_str(&format!("{INDENT}{}: {annotation}\n", field.attribute));
        } else {
            text.push_str(&format!("{INDENT}{}: {annotation} | None = None\n", field.attribute));
        }
    }
    text
}

/// The comment above a field: its doc, the protobuf message of a Proto field, and its JSON key when the attribute
/// is spelled differently.
fn field_comment(field: &PythonField) -> String {
    let mut parts = Vec::new();
    let doc = field.description.doc.replace(['\r', '\n'], " ");
    if !doc.is_empty() {
        parts.push(doc);
    }
    if let FieldKind::Proto(message) = &field.description.kind {
        parts.push(format!("Protobuf JSON of {message}."));
    }
    if field.attribute != field.description.name {
        parts.push(format!("JSON key: {}.", field.description.name));
    }
    parts.join(" ")
}

fn render_from_json(description: &TypeDescription, fields: &[PythonField]) -> String {
    let type_name = &description.name;
    let function_name = format!("{}_{FROM_JSON_SUFFIX}", snake_case(type_name));
    let mut text = format!("def {function_name}(data: dict[str, Any]) -> {type_name}:\n");
    text.push_str(&format!("{INDENT}\"\"\"A {type_name} from its JSON object.\"\"\"\n"));
    if fields.is_empty() {
        text.push_str(&format!("{INDENT}return {type_name}()\n"));
        return text;
    }
    text.push_str(&format!("{INDENT}return {type_name}(\n"));
    for field in fields {
        let key = json_string_literal(&field.description.name);
        let read = converted_value(&field.description.kind, &format!("data[{key}]"), FROM_JSON_SUFFIX, 1);
        let value = if field.description.required {
            read
        } else {
            format!("None if {key} not in data else {read}")
        };
        text.push_str(&format!("{DOUBLE_INDENT}{}={value},\n", field.attribute));
    }
    text.push_str(&format!("{INDENT})\n"));
    text
}

fn render_to_json(description: &TypeDescription, fields: &[PythonField]) -> String {
    let type_name = &description.name;
    let function_name = format!("{}_{TO_JSON_SUFFIX}", snake_case(type_name));
    let mut text = format!("def {function_name}(value: {type_name}) -> dict[str, Any]:\n");
    text.push_str(&format!(
        "{INDENT}\"\"\"The JSON object of a {type_name}; optional fields that are None are left out.\"\"\"\n"
    ));
    text.push_str(&format!("{INDENT}data: dict[str, Any] = {{}}\n"));
    for field in fields {
        let key = json_string_literal(&field.description.name);
        let written = converted_value(
            &field.description.kind,
            &format!("value.{}", field.attribute),
            TO_JSON_SUFFIX,
            1,
        );
        if field.description.required {
            text.push_str(&format!("{INDENT}data[{key}] = {written}\n"));
        } else {
            text.push_str(&format!("{INDENT}if value.{} is not None:\n", field.attribute));
            text.push_str(&format!("{DOUBLE_INDENT}data[{key}] = {written}\n"));
        }
    }
    text.push_str(&format!("{INDENT}return data\n"));
    text
}

/// The expression that converts `source` in one direction (`from_json` or `to_json`). Plain JSON values pass as
/// they are; a described type goes through its function, and lists and maps of them through a comprehension.
/// `depth` names the loop variables of nested comprehensions apart: `item`, then `item_2`.
fn converted_value(kind: &FieldKind, source: &str, suffix: &str, depth: usize) -> String {
    if !holds_a_described_type(kind) {
        return source.to_string();
    }
    match kind {
        FieldKind::Type(type_name) => format!("{}_{suffix}({source})", snake_case(type_name)),
        FieldKind::List(element_kind) => {
            let item = loop_variable("item", depth);
            let element = converted_value(element_kind, &item, suffix, depth + 1);
            format!("[{element} for {item} in {source}]")
        }
        FieldKind::Map(member_kind) => {
            let key = loop_variable("key", depth);
            let item = loop_variable("item", depth);
            let member = converted_value(member_kind, &item, suffix, depth + 1);
            format!("{{{key}: {member} for {key}, {item} in {source}.items()}}")
        }
        _ => source.to_string(),
    }
}

fn loop_variable(base: &str, depth: usize) -> String {
    if depth <= 1 {
        base.to_string()
    } else {
        format!("{base}_{depth}")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rastai_formats::describe::types::{OPTIONAL, REQUIRED, enum_of, field, list_of};
    use rastai_formats::header::describe_shared_header;

    fn tree_types() -> Vec<TypeDescription> {
        vec![
            describe_shared_header(),
            TypeDescription {
                name: "Tree".to_string(),
                schema_id: "rastai.tree/1".to_string(),
                doc: "A tree.".to_string(),
                fields: vec![
                    field(
                        "header",
                        FieldKind::Type("SharedHeader".to_string()),
                        REQUIRED,
                        "The header.",
                    ),
                    field("mode", enum_of(&["split", "tabs"]), REQUIRED, "How it splits."),
                    field("children", list_of(FieldKind::Type("Tree".to_string())), OPTIONAL, ""),
                    field(
                        "grid",
                        list_of(list_of(FieldKind::Type("Tree".to_string()))),
                        OPTIONAL,
                        "Rows of trees.",
                    ),
                    field(
                        "by_id",
                        FieldKind::Map(Box::new(FieldKind::Type("Tree".to_string()))),
                        OPTIONAL,
                        "",
                    ),
                    field(
                        "where_from",
                        FieldKind::Proto("rastai.common.v1.HardwareFingerprint".to_string()),
                        OPTIONAL,
                        "Where.",
                    ),
                    field("from", FieldKind::String, OPTIONAL, "A reserved word."),
                ],
            },
        ]
    }

    fn tree_module_text() -> String {
        let all_types = tree_types();
        let modules = group_types_into_modules(&all_types).unwrap();
        let tree = modules.iter().find(|module| module.name == "tree").unwrap();
        render_python_module(tree, &all_types).unwrap()
    }

    #[test]
    fn names_follow_python_rules() {
        assert_eq!(snake_case("SharedHeader"), "shared_header");
        assert_eq!(snake_case("ModelLoadArgs"), "model_load_args");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("EvalResultV2"), "eval_result_v2");
        assert_eq!(python_field_name("$schema").as_deref(), Some("schema"));
        assert_eq!(python_field_name("from").as_deref(), Some("from_"));
        assert_eq!(python_field_name("type").as_deref(), Some("type"));
        assert_eq!(python_field_name("not-a-name"), None);
        assert_eq!(python_field_name("1st"), None);
    }

    #[test]
    fn the_dataclass_is_frozen_keyword_only_and_keeps_field_order() {
        let text = tree_module_text();
        let expected = "@dataclass(frozen=True, kw_only=True)
class Tree:
    \"A tree.\"

    # The header.
    header: SharedHeader
    # How it splits.
    mode: Literal[\"split\", \"tabs\"]
    children: list[Tree] | None = None
    # Rows of trees.
    grid: list[list[Tree]] | None = None
    by_id: dict[str, Tree] | None = None
    # Where. Protobuf JSON of rastai.common.v1.HardwareFingerprint.
    where_from: dict[str, Any] | None = None
    # A reserved word. JSON key: from.
    from_: str | None = None
";
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("\nfrom typing import Any, Literal\n"));
        assert!(text.contains(
            "\nfrom rastai_formats.generated.shared import SharedHeader, shared_header_from_json, shared_header_to_json\n"
        ));
    }

    #[test]
    fn from_json_converts_nested_types_and_leaves_absent_keys_as_none() {
        let text = tree_module_text();
        let expected = "def tree_from_json(data: dict[str, Any]) -> Tree:
    \"\"\"A Tree from its JSON object.\"\"\"
    return Tree(
        header=shared_header_from_json(data[\"header\"]),
        mode=data[\"mode\"],
        children=None if \"children\" not in data else [tree_from_json(item) for item in data[\"children\"]],
        grid=None if \"grid\" not in data else [[tree_from_json(item_2) for item_2 in item] for item in data[\"grid\"]],
        by_id=None if \"by_id\" not in data else {key: tree_from_json(item) for key, item in data[\"by_id\"].items()},
        where_from=None if \"where_from\" not in data else data[\"where_from\"],
        from_=None if \"from\" not in data else data[\"from\"],
    )
";
        assert!(text.contains(expected), "{text}");
    }

    #[test]
    fn to_json_writes_required_keys_and_present_optional_ones() {
        let text = tree_module_text();
        let expected = "def tree_to_json(value: Tree) -> dict[str, Any]:
    \"\"\"The JSON object of a Tree; optional fields that are None are left out.\"\"\"
    data: dict[str, Any] = {}
    data[\"header\"] = shared_header_to_json(value.header)
    data[\"mode\"] = value.mode
    if value.children is not None:
        data[\"children\"] = [tree_to_json(item) for item in value.children]
";
        assert!(text.contains(expected), "{text}");
        assert!(
            text.contains("    if value.from_ is not None:\n        data[\"from\"] = value.from_\n    return data\n")
        );
    }

    #[test]
    fn the_shared_header_maps_dollar_schema_to_schema() {
        let all_types = vec![describe_shared_header()];
        let modules = group_types_into_modules(&all_types).unwrap();
        let text = render_python_module(&modules[0], &all_types).unwrap();
        assert!(text.contains("    # URL of this document's JSON Schema. JSON key: $schema.\n    schema: str\n"));
        assert!(text.contains("        schema=data[\"$schema\"],\n"));
        assert!(text.contains("    data[\"$schema\"] = value.schema\n"));
        assert!(!text.contains("Literal"));
    }

    #[test]
    fn two_keys_with_one_attribute_name_are_refused() {
        let all_types = vec![TypeDescription {
            name: "Clash".to_string(),
            schema_id: "rastai.clash/1".to_string(),
            doc: String::new(),
            fields: vec![
                field("schema", FieldKind::String, REQUIRED, ""),
                field("$schema", FieldKind::String, REQUIRED, ""),
            ],
        }];
        let modules = group_types_into_modules(&all_types).unwrap();
        let error = render_python_module(&modules[0], &all_types).unwrap_err();
        assert!(error.problem.contains("gives the attribute `schema` twice"), "{error}");
    }

    #[test]
    fn an_enum_with_no_values_is_refused() {
        let all_types = vec![TypeDescription {
            name: "Empty".to_string(),
            schema_id: "rastai.empty/1".to_string(),
            doc: String::new(),
            fields: vec![field("modes", list_of(FieldKind::Enum(Vec::new())), REQUIRED, "")],
        }];
        let modules = group_types_into_modules(&all_types).unwrap();
        let error = render_python_module(&modules[0], &all_types).unwrap_err();
        assert!(
            error.problem.contains("Empty.modes is an enum with no values"),
            "{error}"
        );
    }

    #[test]
    fn init_re_exports_every_class_and_function() {
        let all_types = tree_types();
        let modules = group_types_into_modules(&all_types).unwrap();
        let text = render_python_init(&modules);
        assert!(text.contains(
            "from rastai_formats.generated.shared import SharedHeader, shared_header_from_json, shared_header_to_json\n"
        ));
        assert!(text.contains("from rastai_formats.generated.tree import Tree, tree_from_json, tree_to_json\n"));
        assert!(text.ends_with("    \"tree_from_json\",\n    \"tree_to_json\",\n]\n"));
        assert!(render_python_init(&[]).ends_with("\n__all__: list[str] = []\n"));
    }
}
