//! The list of every described type. The codegen walks it to write `schemas/` and the JavaScript and Python types
//! (layout 4.9), the names registry reads the document schema ids from it (contracts 12.6), and layer 2 looks up
//! `FieldKind::Type` references in it.
//!
//! Each document type's PR adds its file's descriptions here: the document itself and every part of it. The core
//! commands' argument and result types come from `commands::describe_command_types` (M1-22).

use crate::commands::describe_command_types;
use crate::describe::types::TypeDescription;
use crate::documents::lock::describe_lock_types;
use crate::documents::plugin_manifest::describe_plugin_manifest_types;
use crate::documents::project::describe_project_types;
use crate::documents::resume::describe_resume_types;
use crate::documents::run_record::describe_run_record_types;
use crate::documents::settings::describe_settings_types;
use crate::documents::theme::describe_theme_types;
use crate::header::describe_shared_header;

/// Every described type: the shared header first, then each document type's descriptions in `DOCUMENT_TYPES` order,
/// then the command types.
pub fn describe_all_types() -> Vec<TypeDescription> {
    let mut all_types = vec![describe_shared_header()];
    all_types.extend(describe_run_record_types());
    all_types.extend(describe_resume_types());
    all_types.extend(describe_settings_types());
    all_types.extend(describe_project_types());
    all_types.extend(describe_lock_types());
    all_types.extend(describe_plugin_manifest_types());
    all_types.extend(describe_theme_types());
    all_types.extend(describe_command_types());
    all_types
}

/// The description with this name, or None. A linear scan: there are a few dozen types, and a lookup happens once
/// per `FieldKind::Type` value checked.
pub fn find_description<'a>(all_types: &'a [TypeDescription], name: &str) -> Option<&'a TypeDescription> {
    all_types.iter().find(|description| description.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::describe::types::FieldKind;
    use crate::header::SHARED_HEADER_TYPE_NAME;

    /// The type names a kind refers to, nested kinds included.
    fn referenced_types(kind: &FieldKind, names: &mut Vec<String>) {
        match kind {
            FieldKind::Type(name) => names.push(name.clone()),
            FieldKind::List(inner) | FieldKind::Map(inner) => referenced_types(inner, names),
            _ => {}
        }
    }

    #[test]
    fn the_header_is_registered() {
        let all_types = describe_all_types();
        assert!(find_description(&all_types, SHARED_HEADER_TYPE_NAME).is_some());
        assert!(find_description(&all_types, "NoSuchType").is_none());
    }

    #[test]
    fn names_are_unique() {
        let all_types = describe_all_types();
        for description in &all_types {
            let count = all_types.iter().filter(|other| other.name == description.name).count();
            assert_eq!(count, 1, "{} is registered {count} times", description.name);
        }
    }

    // A reference to a type that isn't registered would make layer 2 skip that value and the JSON Schema point at
    // nothing, so it is caught here instead.
    #[test]
    fn every_type_reference_resolves() {
        let all_types = describe_all_types();
        for description in &all_types {
            for field in &description.fields {
                let mut names = Vec::new();
                referenced_types(&field.kind, &mut names);
                for name in names {
                    let found = find_description(&all_types, &name);
                    assert!(found.is_some(), "{}.{} refers to {name}", description.name, field.name);
                }
            }
        }
    }
}
