//! The typed command wrappers: `packages/client/src/generated/commands.js` (layout rust-control-plane.md 4.9,
//! frontend-and-plugins.md 3.2). One plain function per core command, named by the camelCase of its id's segments
//! (`model.load` is `modelLoad`, contracts 6.1), that hands the call to any `CommandExecutor` (owner decision N5):
//! `modelLoad(executor, args)` works with a `DaemonClient`, a `PluginHost` or the mock host alike.
//!
//! The file imports nothing at run time; its types are JSDoc imports: `CommandExecutor` from `@rastai/client`'s
//! command-executor.js, `ExecuteOutcome` from its types.js and each argument type from `@rastai/formats`. With no
//! commands it names none of them, so it type-checks before those files exist (M1-50 writes them).

use std::path::Path;

use rastai_core::commands::naming::javascript_wrapper_name;

use crate::command_source::CommandDescription;
use crate::error::CodegenError;
use crate::javascript_types::jsdoc_text;
use crate::literals::{FORMATS_SOURCE_NOTE, GENERATED_NOTICE, json_string_literal};
use crate::write_if_changed::write_if_changed;

const COMMANDS_MODULE_FILE: &str = "packages/client/src/generated/commands.js";
// Where the wrappers' JSDoc types come from, relative to the generated folder (frontend layout 3.1).
const COMMAND_EXECUTOR_MODULE: &str = "../command-executor.js";
const CLIENT_TYPES_MODULE: &str = "../types.js";
const FORMATS_PACKAGE: &str = "@rastai/formats";
const NO_COMMANDS_LINE: &str = "// No core commands yet: the command table arrives with rastai-core (M1-21, M1-22).\n";

/// Writes commands.js under `output_root`. Returns how many files changed (0 or 1).
pub fn write_javascript_commands(output_root: &Path, commands: &[CommandDescription]) -> Result<usize, CodegenError> {
    let changed = write_if_changed(
        &output_root.join(COMMANDS_MODULE_FILE),
        &render_javascript_commands(commands),
    )?;
    Ok(usize::from(changed))
}

/// The whole module: notice, the type imports the wrappers name, then one wrapper per command.
pub fn render_javascript_commands(commands: &[CommandDescription]) -> String {
    let mut text = format!("// {GENERATED_NOTICE}\n// {FORMATS_SOURCE_NOTE}\n\n");
    if commands.is_empty() {
        text.push_str(NO_COMMANDS_LINE);
        text.push_str("\nexport {};\n");
        return text;
    }
    text.push_str(&type_import_lines(commands));
    for command in commands {
        text.push('\n');
        text.push_str(&render_wrapper(command));
    }
    text
}

/// One `@typedef {import(...)}` line per type the wrappers name: the executor, the outcome, and each distinct
/// argument type in the order the commands first use it.
fn type_import_lines(commands: &[CommandDescription]) -> String {
    let mut text =
        format!("/** @typedef {{import(\"{COMMAND_EXECUTOR_MODULE}\").CommandExecutor}} CommandExecutor */\n");
    text.push_str(&format!(
        "/** @typedef {{import(\"{CLIENT_TYPES_MODULE}\").ExecuteOutcome}} ExecuteOutcome */\n"
    ));
    let mut imported: Vec<&str> = Vec::new();
    for command in commands {
        let type_name = command.arguments_type.as_str();
        if !imported.contains(&type_name) {
            imported.push(type_name);
            text.push_str(&format!(
                "/** @typedef {{import(\"{FORMATS_PACKAGE}\").{type_name}}} {type_name} */\n"
            ));
        }
    }
    text
}

/// One wrapper, as in contracts 12.4: its doc line says the title and the stability. Its name comes from
/// rastai-core's `javascript_wrapper_name`, the one place the camelCase rule lives (layout 10.3).
fn render_wrapper(command: &CommandDescription) -> String {
    let title = command.title.trim_end_matches('.');
    format!(
        "/**
 * {}. Stability: {}.
 * @param {{CommandExecutor}} executor DaemonClient, PluginHost or any other CommandExecutor
 * @param {{{}}} args
 * @returns {{Promise<ExecuteOutcome>}}
 */
export function {}(executor, args) {{
  return executor.execute({}, args);
}}
",
        jsdoc_text(title),
        jsdoc_text(&command.stability),
        command.arguments_type,
        javascript_wrapper_name(&command.command_id),
        json_string_literal(&command.command_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(command_id: &str, arguments_type: &str) -> CommandDescription {
        CommandDescription {
            command_id: command_id.to_string(),
            title: "Load model".to_string(),
            kind: "job".to_string(),
            stability: "preview".to_string(),
            since_version: "0.1.0".to_string(),
            arguments_type: arguments_type.to_string(),
            result_type: format!("{arguments_type}Result"),
        }
    }

    #[test]
    fn a_wrapper_is_named_by_the_core_naming_rule() {
        let text =
            render_javascript_commands(&[command("eval.suite.add_conversation", "EvalSuiteAddConversationArgs")]);
        let name = javascript_wrapper_name("eval.suite.add_conversation");
        assert_eq!(name, "evalSuiteAddConversation");
        assert!(
            text.contains(&format!("export function {name}(executor, args)")),
            "{text}"
        );
    }

    #[test]
    fn no_commands_gives_a_module_that_names_no_type() {
        let text = render_javascript_commands(&[]);
        assert!(text.starts_with(&format!("// {GENERATED_NOTICE}\n")));
        assert!(text.ends_with(&format!("{NO_COMMANDS_LINE}\nexport {{}};\n")));
        assert!(!text.contains("import("));
    }

    #[test]
    fn a_wrapper_matches_the_contracts_example() {
        let text = render_javascript_commands(&[command("model.load", "ModelLoadArgs")]);
        let expected = r#"/** @typedef {import("../command-executor.js").CommandExecutor} CommandExecutor */
/** @typedef {import("../types.js").ExecuteOutcome} ExecuteOutcome */
/** @typedef {import("@rastai/formats").ModelLoadArgs} ModelLoadArgs */

/**
 * Load model. Stability: preview.
 * @param {CommandExecutor} executor DaemonClient, PluginHost or any other CommandExecutor
 * @param {ModelLoadArgs} args
 * @returns {Promise<ExecuteOutcome>}
 */
export function modelLoad(executor, args) {
  return executor.execute("model.load", args);
}
"#;
        assert!(text.ends_with(expected), "{text}");
    }

    #[test]
    fn a_shared_argument_type_is_imported_once() {
        let text = render_javascript_commands(&[command("job.get", "JobArgs"), command("job.cancel", "JobArgs")]);
        assert_eq!(
            text.matches("@typedef {import(\"@rastai/formats\").JobArgs} JobArgs")
                .count(),
            1
        );
        assert!(text.contains("export function jobGet(executor, args)"));
        assert!(text.contains("export function jobCancel(executor, args)"));
    }

    #[test]
    fn no_typescript_syntax_is_written() {
        let text = render_javascript_commands(&[command("model.load", "ModelLoadArgs")]);
        for typescript_only in ["interface ", "export type ", "args: ", "): Promise", " as ", "<T>"] {
            assert!(!text.contains(typescript_only), "{typescript_only}");
        }
    }
}
