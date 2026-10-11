//! The list of every described type. The codegen walks it to write `schemas/` and the JavaScript and Python types
//! (layout 4.9), the names registry reads the document schema ids from it (contracts 12.6), and layer 2 looks up
//! `FieldKind::Type` references in it.
//!
//! Each document type's PR adds its `describe_<type>()` calls here (M1-17 adds the step-1 documents).

use crate::describe::types::TypeDescription;
use crate::header::describe_shared_header;

/// Every described type, the shared header first.
pub fn describe_all_types() -> Vec<TypeDescription> {
    vec![describe_shared_header()]
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
