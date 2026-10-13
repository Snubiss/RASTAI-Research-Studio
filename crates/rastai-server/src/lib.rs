//! rastai-server: the daemon (layout: rust-control-plane.md section 8). M1-26 lands the startup sequence of
//! flows/platform.md 2.1, the HTTP listeners, the request gate of 4.2, one-time login codes and origin-bound
//! sessions (4.1), clean shutdown, `/api/health` and the Connect JSON of the `CommandService` messages (11.3). M1-27
//! adds the command layer: `execute_command` with its scope check, argument check, idempotency and journal, `dispatch`
//! with the first handlers and the L0 stubs, and `Execute` and `ListCommands` over Connect. M1-32 adds the plugin
//! host: discovery, capability negotiation, registration and plugin states, with `plugin.list`. M1-28 adds the bus and
//! `Subscribe`, the daemon's own event log with its replay at startup, and the run tails that index run logs. M1-47
//! adds the embedded node and the node links, `NodeHubService`, the scheduler, and `train.start`, `job.*` and
//! `node.*`.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod auth_files;
pub mod commands;
pub mod daemon;
pub mod error;
pub mod events;
pub mod http;
pub mod indexing;
pub mod nodes;
pub mod plugin_host;
pub mod scheduler;
pub mod server_state;
pub mod shutdown;
