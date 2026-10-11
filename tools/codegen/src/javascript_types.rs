//! The JavaScript types output: `packages/formats/src/generated/`, the types behind `@rastai/formats` (layout
//! rust-control-plane.md 4.9). Plain JavaScript with JSDoc, never TypeScript (ADR-0036): one module per schema id
//! (type_modules.rs) holding one `@typedef` per type with plain `@property` lines, and `index.js` re-exporting them
//! all. Fields keep the JSON's snake_case (contracts 12.4).
//!
//! The JSDoc stays plain: enums are string-literal unions, a protobuf field is a plain object whose doc names its
//! message (contracts 4), and a type from another module is named with an inline `import(...)`, so no module
//! re-declares another's type and `export *` in index.js never sees one name twice.

use std::path::Path;

use rastai_formats::describe::types::{FieldDescription, FieldKind, TypeDescription};

use crate::error::{CodegenError, codegen_error};
use crate::literals::{FORMATS_SOURCE_NOTE, GENERATED_NOTICE, json_string_literal};
use crate::type_modules::{DESCRIPTIONS_SOURCE_FILE, TypeModule, group_types_into_modules, module_of_type};
use crate::write_if_changed::{remove_unwritten_files, write_if_changed};

const TYPES_FOLDER: &str = "packages/formats/src/generated";
const INDEX_MODULE_FILE: &str = "index.js";
const JAVASCRIPT_EXTENSION: &str = ".js";

/// Writes every type module and index.js under `output_root`, then deletes the modules it no longer writes (the
/// folder is this generator's alone). Returns how many files changed or were deleted.
pub fn write_javascript_types(output_root: &Path, all_types: &[TypeDescription]) -> Result<usize, CodegenError> {
    let modules = group_types_into_modules(all_types)?;
    let folder = output_root.join(TYPES_FOLDER);
    let mut changed_count = 0;
    let mut written_paths = Vec::new();
    for module in &modules {
        let contents = render_type_module(module, all_types)?;
        let path = folder.join(javascript_file_name(&module.name));
        if write_if_changed(&path, &contents)? {
            changed_count += 1;
        }
        written_paths.push(path);
    }
    let index_path = folder.join(INDEX_MODULE_FILE);
    if write_if_changed(&index_path, &render_types_index(&modules))? {
        changed_count += 1;
    }
    written_paths.push(index_path);
    changed_count += remove_unwritten_files(&folder, &written_paths)?;
    Ok(changed_count)
}

/// The file of a module: snake_case becomes kebab-case (contracts 12.4), `run_record` is `run-record.js`.
pub fn javascript_file_name(module_name: &str) -> String {
    format!("{}{JAVASCRIPT_EXTENSION}", module_name.replace('_', "-"))
}

/// index.js: one `export *` line per module, so `@rastai/formats` exposes every type by name.
pub fn render_types_index(modules: &[TypeModule]) -> String {
    let mut text = notice_lines();
    text.push_str("// Every generated type module. A type is named as `import(\"@rastai/formats\").<Name>`.\n\n");
    for module in modules {
        text.push_str(&format!(
            "export * from \"./{}\";\n",
            javascript_file_name(&module.name)
        ));
    }
    if modules.is_empty() {
        text.push_str("export {};\n");
    }
    text
}

/// One module: the notice, one typedef per type, then `export {}` so the file is an ES module even though it holds
/// only types.
pub fn render_type_module(module: &TypeModule, all_types: &[TypeDescription]) -> Result<String, CodegenError> {
    let mut text = notice_lines();
    for description in &module.types {
        text.push_str(&render_typedef(description, &module.name, all_types)?);
        text.push('\n');
    }
    text.push_str("export {};\n");
    Ok(text)
}

fn notice_lines() -> String {
    format!("// {GENERATED_NOTICE}\n// {FORMATS_SOURCE_NOTE}\n\n")
}

/// One `@typedef` block: the type's doc, then one `@property` line per field, optional ones in brackets.
fn render_typedef(
    description: &TypeDescription,
    module_name: &str,
    all_types: &[TypeDescription],
) -> Result<String, CodegenError> {
    let mut text = String::from("/**\n");
    if !description.doc.is_empty() {
        text.push_str(&format!(" * {}\n", jsdoc_text(&description.doc)));
    }
    text.push_str(&format!(" * @typedef {{object}} {}\n", description.name));
    for field in &description.fields {
        text.push_str(&render_property(description, field, module_name, all_types)?);
    }
    text.push_str(" */\n");
    Ok(text)
}

fn render_property(
    description: &TypeDescription,
    field: &FieldDescription,
    module_name: &str,
    all_types: &[TypeDescription],
) -> Result<String, CodegenError> {
    if !is_javascript_identifier(&field.name) {
        let problem = format!(
            "{}.{} can't be a JSDoc property name; JSON keys must be identifiers such as snake_case words",
            description.name, field.name
        );
        return Err(codegen_error(Path::new(DESCRIPTIONS_SOURCE_FILE), problem));
    }
    let type_text = javascript_type(&field.kind, module_name, all_types)?;
    let name_text = if field.required {
        field.name.clone()
    } else {
        format!("[{}]", field.name)
    };
    let doc = property_doc(field);
    if doc.is_empty() {
        return Ok(format!(" * @property {{{type_text}}} {name_text}\n"));
    }
    Ok(format!(" * @property {{{type_text}}} {name_text} {doc}\n"))
}

/// The field's doc, plus the message name for a protobuf field (it is a plain object in JavaScript).
fn property_doc(field: &FieldDescription) -> String {
    let doc = jsdoc_text(&field.doc);
    match &field.kind {
        FieldKind::Proto(message) if doc.is_empty() => format!("Protobuf JSON of {message}."),
        FieldKind::Proto(message) => format!("{doc} Protobuf JSON of {message}."),
        _ => doc,
    }
}

/// The JSDoc type of a field kind. A described type from another module is an inline import of that module.
pub fn javascript_type(
    kind: &FieldKind,
    module_name: &str,
    all_types: &[TypeDescription],
) -> Result<String, CodegenError> {
    let text = match kind {
        FieldKind::String => "string".to_string(),
        FieldKind::Integer | FieldKind::Number => "number".to_string(),
        FieldKind::Boolean => "boolean".to_string(),
        FieldKind::Json => "any".to_string(),
        FieldKind::Proto(_) => "object".to_string(),
        FieldKind::Enum(values) => enum_union(values),
        FieldKind::List(element_kind) => list_type(&javascript_type(element_kind, module_name, all_types)?),
        FieldKind::Map(member_kind) => {
            format!(
                "Object<string, {}>",
                javascript_type(member_kind, module_name, all_types)?
            )
        }
        FieldKind::Type(type_name) => {
            let type_module = module_of_type(type_name, all_types)?;
            if type_module == module_name {
                type_name.clone()
            } else {
                format!("import(\"./{}\").{type_name}", javascript_file_name(&type_module))
            }
        }
    };
    Ok(text)
}

/// `"light" | "dark"`. An enum with no values can hold nothing, which JSDoc spells `never`.
fn enum_union(values: &[String]) -> String {
    if values.is_empty() {
        return "never".to_string();
    }
    let literals: Vec<String> = values.iter().map(|value| json_string_literal(value)).collect();
    literals.join(" | ")
}

/// `string[]`, with parentheses around a union so the brackets apply to all of it: `("a" | "b")[]`.
fn list_type(element_type: &str) -> String {
    if element_type.contains(" | ") || element_type.starts_with("import(") {
        format!("({element_type})[]")
    } else {
        format!("{element_type}[]")
    }
}

/// Doc text that is safe inside a `/** ... */` comment on one line: line breaks become spaces and a `*/` can't
/// close the comment early.
pub fn jsdoc_text(text: &str) -> String {
    text.replace(['\r', '\n'], " ").replace("*/", "*\\/")
}

/// An ASCII identifier: a letter, `_` or `$`, then letters, digits, `_` or `$`. Every JSON key Rastai describes is
/// one (`$schema`, `schema_version`), so anything else is a description mistake, not something to quote.
pub fn is_javascript_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let is_start = |character: char| character.is_ascii_alphabetic() || character == '_' || character == '$';
    is_start(first) && characters.all(|character| is_start(character) || character.is_ascii_digit())
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
                doc: "A tree, see */ here.".to_string(),
                fields: vec![
                    field(
                        "header",
                        FieldKind::Type("SharedHeader".to_string()),
                        REQUIRED,
                        "The header.",
                    ),
                    field("mode", enum_of(&["split", "tabs"]), REQUIRED, "How it splits."),
                    field("modes", list_of(enum_of(&["a", "b"])), OPTIONAL, "Several."),
                    field("children", list_of(FieldKind::Type("Tree".to_string())), OPTIONAL, ""),
                    field(
                        "sizes",
                        FieldKind::Map(Box::new(FieldKind::Integer)),
                        OPTIONAL,
                        "Per id.",
                    ),
                    field(
                        "where_from",
                        FieldKind::Proto("rastai.common.v1.HardwareFingerprint".to_string()),
                        OPTIONAL,
                        "Where.",
                    ),
                    field("extra", FieldKind::Json, OPTIONAL, "Line one\nline two."),
                ],
            },
        ]
    }

    fn tree_module(all_types: &[TypeDescription]) -> TypeModule {
        group_types_into_modules(all_types)
            .unwrap()
            .into_iter()
            .find(|module| module.name == "tree")
            .unwrap()
    }

    #[test]
    fn a_typedef_has_one_plain_property_line_per_field() {
        let all_types = tree_types();
        let text = render_type_module(&tree_module(&all_types), &all_types).unwrap();
        let expected = "/**
 * A tree, see *\\/ here.
 * @typedef {object} Tree
 * @property {import(\"./shared.js\").SharedHeader} header The header.
 * @property {\"split\" | \"tabs\"} mode How it splits.
 * @property {(\"a\" | \"b\")[]} [modes] Several.
 * @property {Tree[]} [children]
 * @property {Object<string, number>} [sizes] Per id.
 * @property {object} [where_from] Where. Protobuf JSON of rastai.common.v1.HardwareFingerprint.
 * @property {any} [extra] Line one line two.
 */

export {};
";
        assert!(text.starts_with(&format!("// {GENERATED_NOTICE}\n// {FORMATS_SOURCE_NOTE}\n\n")));
        assert!(text.ends_with(expected), "{text}");
    }

    #[test]
    fn the_shared_header_module_lists_every_header_field() {
        let all_types = vec![describe_shared_header()];
        let modules = group_types_into_modules(&all_types).unwrap();
        let text = render_type_module(&modules[0], &all_types).unwrap();
        assert!(text.contains(" * @typedef {object} SharedHeader\n"));
        assert!(text.contains(" * @property {string} $schema URL of this document's JSON Schema.\n"));
        assert!(text.contains(" * @property {number} schema_version "));
        assert!(text.contains(" * @property {string[]} [derived_from] "));
        assert!(text.contains(" * @property {Object<string, string>} [requires] "));
        assert_eq!(text.matches(" * @property ").count(), 11);
    }

    #[test]
    fn index_re_exports_every_module_in_kebab_case() {
        let all_types = vec![
            describe_shared_header(),
            TypeDescription {
                name: "RunRecord".to_string(),
                schema_id: "rastai.run_record/1".to_string(),
                doc: String::new(),
                fields: Vec::new(),
            },
        ];
        let modules = group_types_into_modules(&all_types).unwrap();
        let text = render_types_index(&modules);
        assert!(
            text.ends_with("export * from \"./run-record.js\";\nexport * from \"./shared.js\";\n"),
            "{text}"
        );
        assert!(render_types_index(&[]).ends_with("export {};\n"));
    }

    #[test]
    fn a_key_that_is_not_an_identifier_is_an_error() {
        let all_types = vec![TypeDescription {
            name: "Bad".to_string(),
            schema_id: "rastai.bad/1".to_string(),
            doc: String::new(),
            fields: vec![field("not-a-name", FieldKind::String, REQUIRED, "")],
        }];
        let modules = group_types_into_modules(&all_types).unwrap();
        let error = render_type_module(&modules[0], &all_types).unwrap_err();
        assert!(error.problem.starts_with("Bad.not-a-name"), "{error}");
    }

    #[test]
    fn identifiers_are_ascii_words_with_dollar_and_underscore() {
        for name in ["$schema", "schema_version", "_private", "a1"] {
            assert!(is_javascript_identifier(name), "{name}");
        }
        for name in ["", "1a", "a-b", "a.b", "é"] {
            assert!(!is_javascript_identifier(name), "{name}");
        }
    }

    #[test]
    fn no_typescript_syntax_is_written() {
        let all_types = tree_types();
        let text = render_type_module(&tree_module(&all_types), &all_types).unwrap();
        for typescript_only in [
            "interface ",
            "export type ",
            ": string",
            " as const",
            "readonly ",
            "keyof ",
        ] {
            assert!(!text.contains(typescript_only), "{typescript_only}");
        }
    }
}
