//! The `secret` area's types (contracts 6.4, 8.5): the OS keychain entries the daemon keeps, by account name.
//!
//! A secret's value is an argument of `secret.set` and nothing else. Its field is `secret`, so the command journal
//! writes it as `"<redacted>"` (contracts 5.1); the listing returns names only, never values.

use serde::{Deserialize, Serialize};

use crate::commands::{command_type, secret};
use crate::describe::types::{FieldKind, REQUIRED, TypeDescription, field, list_of};

const SET_COMMAND_ID: &str = "secret.set";
const DELETE_COMMAND_ID: &str = "secret.delete";
const LIST_COMMAND_ID: &str = "secret.list";

/// Store a value under a keychain account name, such as `hf_token`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretSetArgs {
    pub name: String,
    pub value: String,
}

/// The account name that was written. The value is never echoed back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretSetResult {
    pub name: String,
}

/// Remove a keychain account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretDeleteArgs {
    pub name: String,
}

/// Whether the account existed and was removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretDeleteResult {
    pub name: String,
    pub removed: bool,
}

/// `secret.list` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretListArgs {}

/// The account names the daemon holds, never their values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretListResult {
    pub names: Vec<String>,
}

pub fn describe_secret_set_args() -> TypeDescription {
    let fields = vec![
        field(
            "name",
            FieldKind::String,
            REQUIRED,
            "The keychain account name, such as hf_token.",
        ),
        secret(field(
            "value",
            FieldKind::String,
            REQUIRED,
            "The secret's value. The journal never holds it.",
        )),
    ];
    command_type(
        "SecretSetArgs",
        SET_COMMAND_ID,
        "Store a secret under an account name.",
        fields,
    )
}

pub fn describe_secret_set_result() -> TypeDescription {
    let fields = vec![field(
        "name",
        FieldKind::String,
        REQUIRED,
        "The account name that was written.",
    )];
    command_type(
        "SecretSetResult",
        SET_COMMAND_ID,
        "The account that was written.",
        fields,
    )
}

pub fn describe_secret_delete_args() -> TypeDescription {
    let fields = vec![field(
        "name",
        FieldKind::String,
        REQUIRED,
        "The keychain account name to remove.",
    )];
    command_type("SecretDeleteArgs", DELETE_COMMAND_ID, "Remove a secret.", fields)
}

pub fn describe_secret_delete_result() -> TypeDescription {
    let fields = vec![
        field("name", FieldKind::String, REQUIRED, "The account name."),
        field(
            "removed",
            FieldKind::Boolean,
            REQUIRED,
            "True when the account existed and was removed.",
        ),
    ];
    command_type(
        "SecretDeleteResult",
        DELETE_COMMAND_ID,
        "The outcome of the removal.",
        fields,
    )
}

pub fn describe_secret_list_args() -> TypeDescription {
    command_type(
        "SecretListArgs",
        LIST_COMMAND_ID,
        "secret.list takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_secret_list_result() -> TypeDescription {
    let fields = vec![field(
        "names",
        list_of(FieldKind::String),
        REQUIRED,
        "Every account name, never a value.",
    )];
    command_type(
        "SecretListResult",
        LIST_COMMAND_ID,
        "The account names the daemon holds, never their values.",
        fields,
    )
}
