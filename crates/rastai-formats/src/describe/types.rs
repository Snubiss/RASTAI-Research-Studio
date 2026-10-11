//! Type descriptions as plain data (contracts 5.4, layout 4.2). Each document type has a hand-written
//! `describe_<type>()` next to its struct that returns one of these. They do two jobs: the codegen writes JSON
//! Schema, JSDoc typedefs and Python dataclasses from them, and layer 2 checks raw JSON against them before serde
//! ever sees it, so every structure problem is reported with its path, all at once.
//!
//! No schema library and no proc macros of our own (owner decision 6): if more than about 20 document types make
//! these tables a recurring review cost, the owner may allowlist `schemars` instead.

/// `field(.., REQUIRED, ..)` reads better than a bare `true`.
pub const REQUIRED: bool = true;
pub const OPTIONAL: bool = false;

/// One described type: a document, a part of one (`EvalTest`), the shared header, or a command's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDescription {
    /// The Rust struct's name, which is also the name `FieldKind::Type` refers to it by.
    pub name: String,
    /// `rastai.eval/1` for a document and its parts; empty for a type no one document owns (the shared header).
    pub schema_id: String,
    pub doc: String,
    /// In the struct's order, which is also the order files are written in (json/pretty.rs).
    pub fields: Vec<FieldDescription>,
}

/// One field of a described type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescription {
    /// The JSON key, exactly as serde writes it (`$schema`, `type`, `schema_version`).
    pub name: String,
    pub kind: FieldKind,
    /// A required field must be present. An optional one may be left out; it is never `null`.
    pub required: bool,
    pub doc: String,
    /// A credential passed once: the command journal writes it as `"<redacted>"` (contracts 5.1).
    pub secret: bool,
    /// Conversation or user-written text: the journal redacts it too, so the archive never holds it (contracts 5.1).
    pub content: bool,
    /// The positional CLI argument this field fills (`rastai eval test <suite>`), if any (`x-rastai-cli-position`).
    pub cli_position: Option<u32>,
}

/// What a field holds. The same kinds as a plugin's `types.json`, plus `Json`, `Enum` and `Proto`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    String,
    /// A JSON number written without a fraction or exponent that fits a 64-bit integer.
    Integer,
    Number,
    Boolean,
    /// Free-form JSON, checked by the meaning layer (op attributes, recipe settings). Any value, `null` included.
    Json,
    /// A string that must be one of these values (a Rust enum with `rename_all = "snake_case"`).
    Enum(Vec<String>),
    List(Box<FieldKind>),
    /// An object with string keys, every value of this kind.
    Map(Box<FieldKind>),
    /// Another described type, by its name.
    Type(String),
    /// A wire type stored as its protobuf JSON, such as `rastai.common.v1.HardwareFingerprint`. Layer 2 only checks
    /// that it is a JSON object; `rastai-formats` can't depend on the proto crate (contracts 4, layout 4.2).
    Proto(String),
}

/// A field with no `secret` or `content` flag and no CLI position, the usual case. Set those with struct update
/// syntax: `FieldDescription { secret: true, ..field(...) }`.
pub fn field(name: &str, kind: FieldKind, required: bool, doc: &str) -> FieldDescription {
    FieldDescription {
        name: name.to_string(),
        kind,
        required,
        doc: doc.to_string(),
        secret: false,
        content: false,
        cli_position: None,
    }
}

pub fn list_of(kind: FieldKind) -> FieldKind {
    FieldKind::List(Box::new(kind))
}

pub fn enum_of(values: &[&str]) -> FieldKind {
    FieldKind::Enum(values.iter().map(|value| value.to_string()).collect())
}

/// How a kind is named in diagnostics: "an integer", "a list", "SharedHeader".
pub fn describe_kind(kind: &FieldKind) -> String {
    match kind {
        FieldKind::String => "a string".to_string(),
        FieldKind::Integer => "an integer".to_string(),
        FieldKind::Number => "a number".to_string(),
        FieldKind::Boolean => "true or false".to_string(),
        FieldKind::Json => "any JSON value".to_string(),
        FieldKind::Enum(values) => format!("one of: {}", values.join(", ")),
        FieldKind::List(_) => "a list".to_string(),
        FieldKind::Map(_) => "an object".to_string(),
        FieldKind::Type(name) | FieldKind::Proto(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_starts_with_no_flags_and_struct_update_sets_them() {
        let plain = field("query", FieldKind::String, REQUIRED, "What to search for.");
        assert!(plain.required);
        assert!(!plain.secret && !plain.content);
        assert_eq!(plain.cli_position, None);
        let secret = FieldDescription {
            secret: true,
            ..field("admin_password", FieldKind::String, REQUIRED, "Used once.")
        };
        assert!(secret.secret);
        assert_eq!(secret.name, "admin_password");
    }

    #[test]
    fn helpers_build_the_nested_kinds() {
        assert_eq!(
            list_of(FieldKind::Integer),
            FieldKind::List(Box::new(FieldKind::Integer))
        );
        assert_eq!(
            enum_of(&["light", "dark"]),
            FieldKind::Enum(vec!["light".to_string(), "dark".to_string()])
        );
    }

    #[test]
    fn kinds_have_readable_names() {
        assert_eq!(describe_kind(&FieldKind::Integer), "an integer");
        assert_eq!(describe_kind(&enum_of(&["a", "b"])), "one of: a, b");
        assert_eq!(describe_kind(&FieldKind::Type("EvalTest".to_string())), "EvalTest");
    }
}
