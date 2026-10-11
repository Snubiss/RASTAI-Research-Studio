//! MCP tools built from the command registry (layout: rust-control-plane.md 7.2), the `tools/list` result, and the
//! `tools/call` reply for an outcome the daemon hands back. Every tool is a registry command, named by
//! `mcp_tool_name`; this crate keeps no tool of its own.
//!
//! This is the one file that reads `CommandDescriptor`, the registry's row type (layout 7.4).

use rastai_core::commands::naming::mcp_tool_name;
use rastai_core::commands::registry::{CommandRegistry, check_scope};
use rastai_core::principal::Principal;
use rastai_diag::diagnostic::Diagnostic;
use rastai_diag::render_basic::render_basic;
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind};
use serde_json::{Map, Value, json};

use crate::jsonrpc::{JsonRpcId, JsonRpcMessage};

/// The scope whose commands only read; their tools say so with `readOnlyHint` (layout 7.2).
const READ_SCOPE: &str = "read";
/// Said beside a job's id: a job command returns at once, and its outcome comes later.
const JOB_STARTED_NOTE: &str =
    "The job runs in the background. Call job__get with this job_id for its state and result.";

/// One MCP tool: a registry command as an MCP client sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    /// `mcp_tool_name` of the command id: `eval__suite__edit`.
    pub name: String,
    pub title: String,
    pub description: String,
    pub command_id: String,
    /// The command's arguments schema; `{"type": "object"}` when the registry has none yet.
    pub input_schema: Value,
    /// The command's result schema, `{job_id}` for a job command; `Value::Null` (left out) when there is none.
    pub output_schema: Value,
    pub read_only: bool,
}

/// What running a tool's command gave, as the daemon hands it back after `execute_command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub ok: bool,
    /// The command's result object as JSON text; empty for a job command.
    pub result_json: String,
    /// Set for a job command: the job that runs the work.
    pub job_id: String,
    /// Why the command failed, when `ok` is false.
    pub diagnostics: Vec<Diagnostic>,
}

/// The MCP tool of one registry command.
pub fn tool_from_command(command: &CommandDescriptor) -> McpTool {
    McpTool {
        name: mcp_tool_name(&command.command_id),
        title: command.title.clone(),
        description: command.summary.clone(),
        command_id: command.command_id.clone(),
        input_schema: input_schema(&command.arguments_schema_json),
        output_schema: output_schema(command),
        read_only: command.scope == READ_SCOPE,
    }
}

/// The tools a principal may call: one per registry command whose scope it holds (`check_scope`, the one scope
/// rule), in registry order. Experimental commands only when `plugins.experimental_apis` is on.
pub fn tools_for_principal(
    registry: &CommandRegistry,
    principal: &Principal,
    include_experimental: bool,
) -> Vec<McpTool> {
    registry
        .commands
        .iter()
        .filter(|command| include_experimental || !command.experimental)
        .filter(|command| check_scope(principal, command).is_ok())
        .map(tool_from_command)
        .collect()
}

/// The `tools/list` result. Every tool fits in one page, so there is no `nextCursor`.
pub fn tools_list_result(tools: &[McpTool]) -> Value {
    let listed: Vec<Value> = tools.iter().map(tool_value).collect();
    json!({ "tools": listed })
}

/// The `tools/call` reply. A result goes in `structuredContent` with the same JSON as one text item; a job's id
/// comes with a note to poll it; a failure is `isError: true` with each diagnostic's code and text, so an agent can
/// act on the code.
pub fn tool_call_reply(request_id: JsonRpcId, outcome: &ToolOutcome) -> JsonRpcMessage {
    let result = if !outcome.ok {
        failed_call_result(&outcome.diagnostics)
    } else if !outcome.job_id.is_empty() {
        job_call_result(&outcome.job_id)
    } else {
        sync_call_result(&outcome.result_json)
    };
    JsonRpcMessage::Response { id: request_id, result }
}

fn tool_value(tool: &McpTool) -> Value {
    let mut object = Map::new();
    object.insert("name".to_string(), Value::String(tool.name.clone()));
    object.insert("title".to_string(), Value::String(tool.title.clone()));
    object.insert("description".to_string(), Value::String(tool.description.clone()));
    object.insert("inputSchema".to_string(), tool.input_schema.clone());
    if !tool.output_schema.is_null() {
        object.insert("outputSchema".to_string(), tool.output_schema.clone());
    }
    object.insert("annotations".to_string(), json!({ "readOnlyHint": tool.read_only }));
    Value::Object(object)
}

/// MCP needs an object schema. The registry's schemas are generated, so anything else means "not filled in yet".
fn input_schema(schema_json: &str) -> Value {
    match object_schema(schema_json) {
        Some(schema) => schema,
        None => json!({ "type": "object" }),
    }
}

fn output_schema(command: &CommandDescriptor) -> Value {
    if command.kind == CommandKind::Job as i32 {
        return json!({
            "type": "object",
            "properties": { "job_id": { "type": "string" } },
            "required": ["job_id"],
        });
    }
    object_schema(&command.result_schema_json).unwrap_or(Value::Null)
}

/// The schema when the text is a JSON object; MCP refuses any other kind of schema.
fn object_schema(schema_json: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(schema_json) {
        Ok(Value::Object(schema)) => Some(Value::Object(schema)),
        _ => None,
    }
}

fn sync_call_result(result_json: &str) -> Value {
    let text = if result_json.is_empty() { "{}" } else { result_json };
    let mut result = Map::new();
    result.insert("content".to_string(), json!([text_item(text)]));
    // structuredContent is an object in MCP; a result of another kind travels as text only.
    if let Some(structured) = object_schema(text) {
        result.insert("structuredContent".to_string(), structured);
    }
    result.insert("isError".to_string(), Value::Bool(false));
    Value::Object(result)
}

fn job_call_result(job_id: &str) -> Value {
    let structured = json!({ "job_id": job_id });
    json!({
        "content": [text_item(&structured.to_string()), text_item(JOB_STARTED_NOTE)],
        "structuredContent": structured,
        "isError": false,
    })
}

/// No `structuredContent`: it would have to match the tool's `outputSchema`, which describes a success.
fn failed_call_result(diagnostics: &[Diagnostic]) -> Value {
    let content: Vec<Value> = diagnostics
        .iter()
        .map(|diagnostic| text_item(&diagnostic_text(diagnostic)))
        .collect();
    json!({ "content": content, "isError": true })
}

/// `RRS-C1005: <the message with its values>`, then the Basic text and the fix.
fn diagnostic_text(diagnostic: &Diagnostic) -> String {
    let basic = render_basic(diagnostic);
    format!("{}: {}\n{}", diagnostic.code, diagnostic.message, basic.trim_end())
}

fn text_item(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::principal::{PrincipalKind, owner_principal};
    use rastai_core::scopes::token_preset_scopes;
    use rastai_diag::diagnostic::diagnostic;

    fn command(command_id: &str, scope: &str, kind: CommandKind) -> CommandDescriptor {
        CommandDescriptor {
            command_id: command_id.to_string(),
            title: format!("{command_id} title"),
            summary: format!("{command_id} summary"),
            scope: scope.to_string(),
            kind: kind as i32,
            ..CommandDescriptor::default()
        }
    }

    fn mcp_default_principal() -> Principal {
        Principal {
            kind: PrincipalKind::Token,
            id: "tok_01J".to_string(),
            scopes: token_preset_scopes("mcp-default").unwrap(),
        }
    }

    fn registry() -> CommandRegistry {
        let mut experimental = command("system.env.list", "read", CommandKind::Sync);
        experimental.experimental = true;
        CommandRegistry {
            commands: vec![
                command("system.status", "read", CommandKind::Sync),
                command("train.start", "training", CommandKind::Job),
                experimental,
                command("acme.tool.run", "plugin:acme", CommandKind::Sync),
            ],
        }
    }

    fn names(tools: &[McpTool]) -> Vec<String> {
        tools.iter().map(|tool| tool.name.clone()).collect()
    }

    #[test]
    fn a_tool_is_its_command_under_the_mcp_name() {
        let mut descriptor = command("eval.suite.edit", "eval", CommandKind::Sync);
        descriptor.arguments_schema_json =
            r#"{"type":"object","properties":{"suite_id":{"type":"string"}}}"#.to_string();
        descriptor.result_schema_json = r#"{"type":"object"}"#.to_string();
        let tool = tool_from_command(&descriptor);
        assert_eq!(tool.name, "eval__suite__edit");
        assert_eq!(tool.command_id, "eval.suite.edit");
        assert_eq!(tool.input_schema["properties"]["suite_id"]["type"], "string");
        assert_eq!(tool.output_schema, json!({"type": "object"}));
        assert!(!tool.read_only);
    }

    #[test]
    fn missing_schemas_give_an_object_input_and_no_output() {
        let tool = tool_from_command(&command("system.status", "read", CommandKind::Sync));
        assert_eq!(tool.input_schema, json!({"type": "object"}));
        assert_eq!(tool.output_schema, Value::Null);
        assert!(tool.read_only);
        let listed = tools_list_result(&[tool]);
        assert!(listed["tools"][0].get("outputSchema").is_none());
        assert_eq!(listed["tools"][0]["annotations"]["readOnlyHint"], true);
    }

    #[test]
    fn a_job_tool_returns_its_job_id() {
        let tool = tool_from_command(&command("train.start", "training", CommandKind::Job));
        assert_eq!(tool.output_schema["required"], json!(["job_id"]));
    }

    #[test]
    fn tools_are_the_commands_the_principal_may_run() {
        let tools = tools_for_principal(&registry(), &mcp_default_principal(), false);
        assert_eq!(names(&tools), ["system__status"]);
        let with_experimental = tools_for_principal(&registry(), &mcp_default_principal(), true);
        assert_eq!(names(&with_experimental), ["system__status", "system__env__list"]);
        let owner_tools = tools_for_principal(&registry(), &owner_principal(), false);
        assert_eq!(
            names(&owner_tools),
            ["system__status", "train__start", "acme__tool__run"]
        );
    }

    #[test]
    fn a_failed_call_carries_each_code() {
        let outcome = ToolOutcome {
            ok: false,
            result_json: String::new(),
            job_id: String::new(),
            diagnostics: vec![diagnostic("RRS-C0010", &[("command_id", "train.start")])],
        };
        let JsonRpcMessage::Response { result, .. } = tool_call_reply(JsonRpcId::Number(3), &outcome) else {
            panic!("a tool call always gets a result");
        };
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("RRS-C0010: train.start isn't available"), "{text}");
        assert!(result.get("structuredContent").is_none());
    }

    #[test]
    fn a_result_that_is_not_an_object_travels_as_text_only() {
        let outcome = ToolOutcome {
            ok: true,
            result_json: "[1,2]".to_string(),
            job_id: String::new(),
            diagnostics: Vec::new(),
        };
        let JsonRpcMessage::Response { result, .. } = tool_call_reply(JsonRpcId::Number(3), &outcome) else {
            panic!("a tool call always gets a result");
        };
        assert_eq!(result["content"][0]["text"], "[1,2]");
        assert!(result.get("structuredContent").is_none());
    }
}
