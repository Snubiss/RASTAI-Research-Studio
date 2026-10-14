//! The `hf` area's types (contracts 6.4, 6.5): the Hugging Face sign-in and the license check of a gated model.
//!
//! The token is an argument, and its field is `secret`: the command journal writes it as `"<redacted>"` (contracts 5.1).

use serde::{Deserialize, Serialize};

use crate::commands::{command_type, secret};
use crate::describe::types::{FieldKind, REQUIRED, TypeDescription, field};

const LOGIN_COMMAND_ID: &str = "hf.login";
const LICENSE_CHECK_COMMAND_ID: &str = "hf.license.check";

/// Store a Hugging Face token in the keychain (account `hf_token`) and verify it with the Hub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfLoginArgs {
    pub token: String,
}

/// The account the token belongs to, once the Hub accepted it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfLoginResult {
    pub username: String,
}

/// Ask the Hub whether the signed-in account has accepted a gated model's license.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfLicenseCheckArgs {
    /// The repository id, such as `meta-llama/Llama-3.1-8B`.
    pub repo_id: String,
}

/// Whether access is granted, and the license the repository names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfLicenseCheckResult {
    pub repo_id: String,
    pub accepted: bool,
    pub license: String,
    /// Empty when the repository names no license page.
    pub license_url: String,
}

pub fn describe_hf_login_args() -> TypeDescription {
    let fields = vec![secret(field(
        "token",
        FieldKind::String,
        REQUIRED,
        "The Hugging Face token. The journal never holds it.",
    ))];
    command_type(
        "HfLoginArgs",
        LOGIN_COMMAND_ID,
        "Store and verify a Hugging Face token.",
        fields,
    )
}

pub fn describe_hf_login_result() -> TypeDescription {
    let fields = vec![field(
        "username",
        FieldKind::String,
        REQUIRED,
        "The account the token belongs to.",
    )];
    command_type(
        "HfLoginResult",
        LOGIN_COMMAND_ID,
        "The account the token signed in as.",
        fields,
    )
}

pub fn describe_hf_license_check_args() -> TypeDescription {
    let fields = vec![field(
        "repo_id",
        FieldKind::String,
        REQUIRED,
        "The repository id, such as meta-llama/Llama-3.1-8B.",
    )];
    command_type(
        "HfLicenseCheckArgs",
        LICENSE_CHECK_COMMAND_ID,
        "Check whether a gated model's license is accepted.",
        fields,
    )
}

pub fn describe_hf_license_check_result() -> TypeDescription {
    let fields = vec![
        field("repo_id", FieldKind::String, REQUIRED, "The repository checked."),
        field(
            "accepted",
            FieldKind::Boolean,
            REQUIRED,
            "True when the signed-in account has access.",
        ),
        field(
            "license",
            FieldKind::String,
            REQUIRED,
            "The license the repository names.",
        ),
        field(
            "license_url",
            FieldKind::String,
            REQUIRED,
            "The license page, or empty when there is none.",
        ),
    ];
    command_type(
        "HfLicenseCheckResult",
        LICENSE_CHECK_COMMAND_ID,
        "Whether the license of a gated model is accepted.",
        fields,
    )
}
