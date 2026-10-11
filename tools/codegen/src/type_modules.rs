//! Which generated module each described type goes into, shared by the JavaScript and Python outputs so both group
//! the types the same way (layout rust-control-plane.md 4.9).
//!
//! One module per schema id: the types of `rastai.eval/1` (`EvalTest`, `Expectation`, ...) all go into `eval`, a
//! command's arguments and result (`rastai.command.model.load/1`) into `command_model_load`, and the types no one
//! document owns (the shared header, whose schema id is empty) into `shared`.

use std::path::Path;

use rastai_formats::describe::registry::find_description;
use rastai_formats::describe::types::{FieldKind, TypeDescription};

use crate::error::{CodegenError, codegen_error};

/// The module for types with an empty schema id.
pub const SHARED_MODULE_NAME: &str = "shared";
/// Every first-party schema id starts with this; it says nothing inside a module name.
const SCHEMA_ID_PREFIX: &str = "rastai.";
/// The file a missing or clashing description has to be fixed in.
pub const DESCRIPTIONS_SOURCE_FILE: &str = "crates/rastai-formats/src/describe/registry.rs";

/// The types that go into one generated module, in the order `describe_all_types` lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeModule {
    /// snake_case, such as `run_record`. The JavaScript file is its kebab-case form, the Python file this name.
    pub name: String,
    /// The schema id every type in the module shares; empty for `shared`.
    pub schema_id: String,
    pub types: Vec<TypeDescription>,
}

/// The module name for a schema id: `rastai.run_record/1` is `run_record`, `rastai.command.model.load/1` is
/// `command_model_load`, and an empty id is `shared`. The version is left out: only the current version of a type
/// is described, so two versions never need a module each.
pub fn module_name_for(schema_id: &str) -> String {
    if schema_id.is_empty() {
        return SHARED_MODULE_NAME.to_string();
    }
    let without_prefix = schema_id.strip_prefix(SCHEMA_ID_PREFIX).unwrap_or(schema_id);
    let without_version = match without_prefix.split_once('/') {
        Some((type_part, _version)) => type_part,
        None => without_prefix,
    };
    without_version.replace('.', "_")
}

/// The types grouped by module, modules sorted by name. Fails when two schema ids would share a module name, since
/// one would silently overwrite the other's file.
pub fn group_types_into_modules(all_types: &[TypeDescription]) -> Result<Vec<TypeModule>, CodegenError> {
    let mut modules: Vec<TypeModule> = Vec::new();
    for description in all_types {
        let name = module_name_for(&description.schema_id);
        match modules.iter_mut().find(|module| module.name == name) {
            Some(module) if module.schema_id == description.schema_id => module.types.push(description.clone()),
            Some(module) => return Err(module_clash_error(module, description)),
            None => modules.push(TypeModule {
                name,
                schema_id: description.schema_id.clone(),
                types: vec![description.clone()],
            }),
        }
    }
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(modules)
}

fn module_clash_error(module: &TypeModule, description: &TypeDescription) -> CodegenError {
    let problem = format!(
        "{} ({}) and the types of {} would both go into the generated module `{}`; rename one schema id",
        description.name, description.schema_id, module.schema_id, module.name
    );
    codegen_error(Path::new(DESCRIPTIONS_SOURCE_FILE), problem)
}

/// The module a referenced type lives in. Fails for a name no description has, which `describe_all_types`'s own
/// test also refuses.
pub fn module_of_type(type_name: &str, all_types: &[TypeDescription]) -> Result<String, CodegenError> {
    match find_description(all_types, type_name) {
        Some(description) => Ok(module_name_for(&description.schema_id)),
        None => Err(codegen_error(
            Path::new(DESCRIPTIONS_SOURCE_FILE),
            format!("a field refers to the type `{type_name}`, which has no description"),
        )),
    }
}

/// The type names a module's fields refer to, nested kinds included, each once, in first-seen order.
pub fn referenced_type_names(module: &TypeModule) -> Vec<String> {
    let mut names = Vec::new();
    for description in &module.types {
        for field in &description.fields {
            collect_type_names(&field.kind, &mut names);
        }
    }
    names
}

fn collect_type_names(kind: &FieldKind, names: &mut Vec<String>) {
    match kind {
        FieldKind::Type(name) => {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        FieldKind::List(inner) | FieldKind::Map(inner) => collect_type_names(inner, names),
        _ => {}
    }
}

/// True when the kind holds a described type somewhere inside, so reading or writing it needs a conversion
/// function rather than taking the JSON value as it is.
pub fn holds_a_described_type(kind: &FieldKind) -> bool {
    match kind {
        FieldKind::Type(_) => true,
        FieldKind::List(inner) | FieldKind::Map(inner) => holds_a_described_type(inner),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rastai_formats::describe::types::{REQUIRED, field, list_of};

    fn described(
        name: &str,
        schema_id: &str,
        fields: Vec<rastai_formats::describe::types::FieldDescription>,
    ) -> TypeDescription {
        TypeDescription {
            name: name.to_string(),
            schema_id: schema_id.to_string(),
            doc: String::new(),
            fields,
        }
    }

    #[test]
    fn module_names_drop_the_prefix_and_version() {
        assert_eq!(module_name_for("rastai.eval/1"), "eval");
        assert_eq!(module_name_for("rastai.run_record/1"), "run_record");
        assert_eq!(module_name_for("rastai.command.model.load/1"), "command_model_load");
        assert_eq!(module_name_for(""), SHARED_MODULE_NAME);
    }

    #[test]
    fn types_of_one_schema_id_share_a_module_and_modules_are_sorted() {
        let all_types = vec![
            described("EvalSuite", "rastai.eval/1", Vec::new()),
            described("SharedHeader", "", Vec::new()),
            described("EvalTest", "rastai.eval/1", Vec::new()),
        ];
        let modules = group_types_into_modules(&all_types).unwrap();
        let names: Vec<&str> = modules.iter().map(|module| module.name.as_str()).collect();
        assert_eq!(names, ["eval", "shared"]);
        let eval_types: Vec<&str> = modules[0].types.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(eval_types, ["EvalSuite", "EvalTest"]);
    }

    #[test]
    fn two_schema_ids_with_one_module_name_are_refused() {
        let all_types = vec![
            described("A", "rastai.run_record/1", Vec::new()),
            described("B", "rastai.run.record/1", Vec::new()),
        ];
        let error = group_types_into_modules(&all_types).unwrap_err();
        assert!(error.problem.contains("`run_record`"), "{error}");
    }

    #[test]
    fn references_are_found_inside_lists_and_maps_once_each() {
        let node = FieldKind::Type("Node".to_string());
        let module = TypeModule {
            name: "tree".to_string(),
            schema_id: "rastai.tree/1".to_string(),
            types: vec![described(
                "Tree",
                "rastai.tree/1",
                vec![
                    field("root", node.clone(), REQUIRED, ""),
                    field("children", list_of(node.clone()), REQUIRED, ""),
                    field("by_id", FieldKind::Map(Box::new(node)), REQUIRED, ""),
                ],
            )],
        };
        assert_eq!(referenced_type_names(&module), ["Node"]);
        assert!(holds_a_described_type(&list_of(FieldKind::Type("Node".to_string()))));
        assert!(!holds_a_described_type(&list_of(FieldKind::String)));
    }

    #[test]
    fn an_unknown_reference_is_an_error() {
        let error = module_of_type("Missing", &[]).unwrap_err();
        assert!(error.problem.contains("`Missing`"), "{error}");
    }
}
