//! Every core command reaches its own `match` arm in dispatch.rs, never the fall-through to the plugin host (layout:
//! rust-control-plane.md 8.6 and 10.1). The fall-through answers a core command with `RRS-C0001` (it has no arm)
//! and a plugin's with `RRS-P1004` (forwarding arrives with the plugin modules, M4), so a core id that comes back
//! with `RRS-C0001` has lost its arm. Every arm runs here as a dry run, so nothing is written or stopped.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use rastai_core::commands::table::COMMANDS;
use rastai_core::principal::owner_principal;
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_server::commands::context::CommandContext;
use rastai_server::commands::dispatch::{dispatch, not_available_code};
use rastai_server::server_state::Server;

use common::{remove_test_folder, start_test_daemon, stop_test_daemon, test_paths};

const FALL_THROUGH_CORE_CODE: &str = "RRS-C0001";
const FALL_THROUGH_PLUGIN_CODE: &str = "RRS-P1004";

fn registered_descriptor(server: &Server, command_id: &str) -> CommandDescriptor {
    let registry = server.registry.read().unwrap();
    let found = registry
        .commands
        .iter()
        .find(|command| command.command_id == command_id);
    found.unwrap().clone()
}

/// The codes `dispatch` answers a descriptor with, run as an owner's dry run with no arguments.
async fn dispatched_codes(server: &Server, command: &CommandDescriptor) -> Vec<String> {
    let principal = owner_principal();
    let context = CommandContext {
        server,
        principal: &principal,
        command,
        journal_id: String::new(),
        project_id: String::new(),
        idempotency_key: String::new(),
        dry_run: true,
    };
    match dispatch(&context, "{}").await {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics.iter().map(|item| item.code.clone()).collect(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_core_command_has_its_own_arm() {
    let paths = test_paths("dispatch");
    let running = start_test_daemon(&paths).await;
    for row in COMMANDS {
        let command = registered_descriptor(&running.server, row.command_id);
        let codes = dispatched_codes(&running.server, &command).await;
        assert!(
            !codes.contains(&FALL_THROUGH_CORE_CODE.to_string()),
            "{} has no arm in dispatch.rs",
            row.command_id
        );
        // A stub's arm answers its own area's code.
        let stub_code = not_available_code(row.command_id).to_string();
        if codes.contains(&stub_code) {
            assert_eq!(codes, [stub_code], "{}", row.command_id);
        }
    }
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn ids_without_an_arm_reach_the_fall_through() {
    let paths = test_paths("fallthru");
    let running = start_test_daemon(&paths).await;
    let core_without_arm = CommandDescriptor {
        command_id: "system.made_up".to_string(),
        ..CommandDescriptor::default()
    };
    let codes = dispatched_codes(&running.server, &core_without_arm).await;
    assert_eq!(codes, [FALL_THROUGH_CORE_CODE]);
    let plugin_command = CommandDescriptor {
        command_id: "acme.tool.run".to_string(),
        plugin_id: "acme".to_string(),
        ..CommandDescriptor::default()
    };
    let codes = dispatched_codes(&running.server, &plugin_command).await;
    assert_eq!(codes, [FALL_THROUGH_PLUGIN_CODE]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
