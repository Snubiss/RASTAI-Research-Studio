//! The core command table (layout rust-control-plane.md 5.5, 10.1): every row of `COMMANDS` is well formed — an
//! id of the contract form in a core area, no duplicates, a fixed scope, a known kind and section, a semver
//! `since_version`, and `preview` stability until Phase 2 picks the stable subset. Retired ids are never reused.
//!
//! `row_problems` holds the checks, so the same function runs over the table and over sample bad rows: an empty
//! table (before M1-22's rows) still proves each check catches what it should.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use rastai_core::commands::descriptor::CommandRow;
use rastai_core::commands::naming::check_command_id;
use rastai_core::commands::table::{COMMANDS, RETIRED_COMMANDS};
use rastai_core::scopes::Scope;
use rastai_formats::semver::parse_version;
use rastai_proto::commands::v1::CommandKind;
use rastai_proto::common::v1::Stability;

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
