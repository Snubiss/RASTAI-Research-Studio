//! The `command` area's types (contracts 6.4): `command.list`, the registry as a command (also `ListCommands`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::command_type;
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

const LIST_COMMAND_ID: &str = "command.list";
/// The registry holds the generated proto descriptor; rastai-formats can't depend on the proto crate, so a
/// descriptor travels as its protobuf JSON (contracts 4, layout 4.2).
const COMMAND_DESCRIPTOR_MESSAGE: &str = "rastai.commands.v1.CommandDescriptor";

/// Which commands to list: the same three filters as `ListCommandsRequest`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandListArgs {
    /// Only this section's commands; empty for every section.
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub include_experimental: bool,
    #[serde(default)]
    pub basic_only: bool,
}

/// The registered commands, core commands first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandListResult {
    /// Each one is a `CommandDescriptor` as protobuf JSON.
    pub commands: Vec<Value>,
}

pub fn describe_command_list_args() -> TypeDescription {
    let fields = vec![
        field(
            "section",
            FieldKind::String,
            OPTIONAL,
            "Only this section's commands; empty or absent for every section.",
        ),
        field(
            "include_experimental",
            FieldKind::Boolean,
            OPTIONAL,
            "Also list experimental commands.",
        ),
        field(
            "basic_only",
            FieldKind::Boolean,
            OPTIONAL,
            "Only the commands shown in Basic mode.",
        ),
    ];
    command_type(
        "CommandListArgs",
        LIST_COMMAND_ID,
        "Which registered commands to list.",
        fields,
    )
}

pub fn describe_command_list_result() -> TypeDescription {
    let fields = vec![field(
        "commands",
        list_of(FieldKind::Proto(COMMAND_DESCRIPTOR_MESSAGE.to_string())),
        REQUIRED,
        "The registered commands, core commands first, with their argument and result schemas.",
    )];
    command_type("CommandListResult", LIST_COMMAND_ID, "The registered commands.", fields)
}
