//! The core command table (layout rust-control-plane.md 5.5, 10.1): every row of `COMMANDS` is well formed — an
//! id of the contract form in a core area, no duplicates, a fixed scope, a known kind and section, a semver
//! `since_version`, and `preview` stability until Phase 2 picks the stable subset. Retired ids are never reused.
//! No argument field takes a global CLI flag (layout 9.4), the table holds the skeleton's commands (M1-22), and no
//! skeleton command that changes something holds `read`.
//!
//! `row_problems` and `flag_collisions` hold the checks, so the same functions run over the table and over sample
//! bad rows and fields, which proves each check catches what it should.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use rastai_core::commands::descriptor::CommandRow;
use rastai_core::commands::naming::{GLOBAL_FLAGS, check_command_id, flag_name, negated_flag_name};
use rastai_core::commands::table::{COMMANDS, RETIRED_COMMANDS};
use rastai_core::scopes::Scope;
use rastai_formats::commands::describe_arguments;
use rastai_formats::describe::types::{FieldDescription, FieldKind, OPTIONAL, TypeDescription, field};
use rastai_formats::semver::parse_version;
use rastai_proto::commands::v1::CommandKind;
use rastai_proto::common::v1::Stability;

/// The M2 commands (plan M2-12, contracts 6.4): honest L0 stubs, registered before their handlers.
const M2_COMMAND_IDS: &[&str] = &[
    "system.hardware",
    "system.env.list",
    "system.env.install",
    "system.doctor",
    "system.bundle",
    "secret.set",
    "secret.delete",
    "secret.list",
    "hf.login",
    "hf.license.check",
    "model.list",
    "model.inspect",
    "model.download",
];

/// The app's section ids (contracts 15.6); a core command sits in one of them.
const CORE_SECTIONS: &[&str] = &[
    "home",
    "inference",
    "training",
    "designer",
    "gym",
    "data",
    "plugins",
    "remote",
    "settings",
];

/// The skeleton's commands (plan M1-22, contracts 6.4): the first rows, every one `preview`.
const SKELETON_COMMAND_IDS: &[&str] = &[
    "system.status",
    "system.shutdown",
    "command.list",
    "job.list",
    "job.get",
    "job.cancel",
    "train.start",
    "settings.get",
    "settings.set",
    "token.login_code.create",
    "token.create",
    "token.list",
    "token.revoke",
    "plugin.list",
    "node.list",
    "node.status",
];

/// The skeleton's commands that change something. The row scope is the line between changing and read-only commands:
/// a `read` command is never journaled, still runs during an update's maintenance window, and every read-only token
/// reaches it (flows/platform.md 3.1, 9.2), so none of these may hold `read`.
const CHANGING_COMMAND_IDS: &[&str] = &[
    "system.shutdown",
    "job.cancel",
    "train.start",
    "settings.set",
    "token.login_code.create",
    "token.create",
    "token.revoke",
];

const GOOD_ROW: CommandRow = CommandRow {
    command_id: "system.status",
    title: "Show status",
    summary: "Daemon, nodes, versions.",
    section: "settings",
    scope: Scope::Read,
    kind: CommandKind::Sync,
    stability: Stability::Preview,
    undoable: false,
    basic: true,
    default_shortcut: "",
    experimental: false,
    since_version: "0.1.0",
};

/// Every way a single row breaks the table's rules, as short descriptions; empty for a good row.
fn row_problems(row: &CommandRow) -> Vec<String> {
    let mut problems = Vec::new();
    if let Err(refusal) = check_command_id(row.command_id, "") {
        problems.push(format!("id: {}", refusal.code));
    }
    if row.title.is_empty() || row.summary.is_empty() {
        problems.push("title or summary empty".to_string());
    }
    if !CORE_SECTIONS.contains(&row.section) {
        problems.push(format!("section {:?}", row.section));
    }
    if matches!(row.scope, Scope::Plugin(_)) {
        problems.push("a plugin's own scope".to_string());
    }
    if row.kind == CommandKind::Unspecified {
        problems.push("kind unspecified".to_string());
    }
    if row.stability != Stability::Preview {
        problems.push("not preview".to_string());
    }
    if parse_version(row.since_version).is_err() {
        problems.push(format!("since_version {:?}", row.since_version));
    }
    problems
}

/// Every flag an argument type's fields would take on the command line that is a global flag: `--name` for each
/// field, and `--no-name` too for a boolean one (layout 9.4).
fn flag_collisions(arguments: &TypeDescription) -> Vec<String> {
    let mut collisions = Vec::new();
    for field in &arguments.fields {
        let mut flags = vec![flag_name(&field.name)];
        if field.kind == FieldKind::Boolean {
            flags.push(negated_flag_name(&field.name));
        }
        for flag in flags {
            if GLOBAL_FLAGS.contains(&flag.as_str()) {
                collisions.push(format!("{}.{} takes {flag}", arguments.name, field.name));
            }
        }
    }
    collisions
}

fn sample_arguments(fields: Vec<FieldDescription>) -> TypeDescription {
    TypeDescription {
        name: "SampleArgs".to_string(),
        schema_id: "rastai.command.sample.run/1".to_string(),
        doc: "Sample arguments.".to_string(),
        fields,
    }
}

#[test]
fn every_row_is_well_formed() {
    for row in COMMANDS {
        let problems = row_problems(row);
        assert!(problems.is_empty(), "{}: {problems:?}", row.command_id);
    }
}

#[test]
fn no_id_is_listed_twice() {
    let mut seen = BTreeSet::new();
    for row in COMMANDS {
        assert!(seen.insert(row.command_id), "{} is listed twice", row.command_id);
    }
}

#[test]
fn retired_ids_are_never_reused() {
    for retired in RETIRED_COMMANDS {
        assert!(
            COMMANDS.iter().all(|row| row.command_id != retired.id),
            "{}",
            retired.id
        );
        assert!(
            COMMANDS.iter().any(|row| row.command_id == retired.replaced_by),
            "{}",
            retired.replaced_by
        );
        assert!(parse_version(retired.retired_in).is_ok(), "{}", retired.retired_in);
    }
}

#[test]
fn the_checks_accept_a_good_row() {
    assert_eq!(row_problems(&GOOD_ROW), Vec::<String>::new());
}

#[test]
fn the_checks_catch_each_bad_field() {
    let bad_rows = [
        CommandRow {
            command_id: "psychology.step",
            ..GOOD_ROW
        },
        CommandRow {
            command_id: "system.Status",
            ..GOOD_ROW
        },
        CommandRow { title: "", ..GOOD_ROW },
        CommandRow {
            section: "system",
            ..GOOD_ROW
        },
        CommandRow {
            kind: CommandKind::Unspecified,
            ..GOOD_ROW
        },
        CommandRow {
            stability: Stability::Stable,
            ..GOOD_ROW
        },
        CommandRow {
            since_version: "",
            ..GOOD_ROW
        },
        CommandRow {
            since_version: "1.0",
            ..GOOD_ROW
        },
    ];
    for row in &bad_rows {
        assert_eq!(row_problems(row).len(), 1, "{row:?}");
    }
    let plugin_scope = CommandRow {
        scope: Scope::Plugin("acme".to_string()),
        ..GOOD_ROW
    };
    assert_eq!(row_problems(&plugin_scope).len(), 1);
}

// The table lists the skeleton's commands first and then each milestone's, in the order they were added, so a later
// milestone's rows never reorder an earlier one's.
#[test]
fn the_skeleton_commands_are_registered() {
    let registered: Vec<&str> = COMMANDS.iter().map(|row| row.command_id).collect();
    assert_eq!(&registered[..SKELETON_COMMAND_IDS.len()], SKELETON_COMMAND_IDS);
    let after_skeleton = &registered[SKELETON_COMMAND_IDS.len()..];
    assert_eq!(after_skeleton, M2_COMMAND_IDS);
}

#[test]
fn no_changing_command_holds_read() {
    for command_id in CHANGING_COMMAND_IDS {
        let row = COMMANDS.iter().find(|row| row.command_id == *command_id).unwrap();
        assert_ne!(row.scope, Scope::Read, "{command_id} changes something but holds read");
    }
}

// Contracts 6.4: a training job is M1's only job kind, so cancelling one takes `training`.
#[test]
fn job_cancel_holds_training() {
    let row = COMMANDS.iter().find(|row| row.command_id == "job.cancel").unwrap();
    assert_eq!(row.scope, Scope::Training);
}

#[test]
fn no_argument_takes_a_global_cli_flag() {
    for row in COMMANDS {
        let arguments = describe_arguments(row.command_id);
        let arguments = arguments.unwrap_or_else(|| panic!("{} has no argument type", row.command_id));
        assert_eq!(flag_collisions(&arguments), Vec::<String>::new());
    }
}

#[test]
fn a_field_that_takes_a_global_flag_is_caught() {
    let fine = sample_arguments(vec![
        field("node_id", FieldKind::String, OPTIONAL, "Not --node."),
        field(
            "dry",
            FieldKind::Boolean,
            OPTIONAL,
            "Neither --dry nor --no-dry is global.",
        ),
    ]);
    assert_eq!(flag_collisions(&fine), Vec::<String>::new());
    let colliding = sample_arguments(vec![
        field("dry_run", FieldKind::Boolean, OPTIONAL, "Takes --dry-run."),
        field("project", FieldKind::String, OPTIONAL, "Takes --project."),
        field("start", FieldKind::Boolean, OPTIONAL, "Its off flag is --no-start."),
    ]);
    assert_eq!(
        flag_collisions(&colliding),
        [
            "SampleArgs.dry_run takes --dry-run",
            "SampleArgs.project takes --project",
            "SampleArgs.start takes --no-start",
        ]
    );
    // A text field named `start` has no off flag, so it is free.
    let text_start = sample_arguments(vec![field("start", FieldKind::String, OPTIONAL, "Takes --start.")]);
    assert_eq!(flag_collisions(&text_start), Vec::<String>::new());
}
