//! How the node reaches a running job (layout: rust-node-gateway-proto.md section 7.4, "Signals"): operating-system
//! signals to the job's whole process group, and `WorkerService.Control` for the values the worker acts on at its
//! next step boundary. Which `JobSignal` takes which path is supervisor.rs's decision; this file only delivers.
//!
//! Rust's std has no `killpg`, and `libc` would need `unsafe`, which contracts 12.2 allows only in ADR-0033's one
//! file. So a group signal runs the `kill` tool (procps): `kill -s <SIGNAL> -- -<pgid>`. Signals are rare, so one
//! process spawn each is fine.
//!
//! Before any group signal the node re-reads the leader's identity from `/proc` (start ticks, process group, boot id)
//! and compares it with what it recorded at spawn, so it never signals a pid that was reused (7.4 "Reattach").
//! Linux only: other systems have no `/proc`, so there the check fails and nothing is signalled (tier 2, spec §28).

use std::path::PathBuf;

use rastai_proto::common::v1::Diagnostic as ProtoDiagnostic;
use rastai_proto::node::v1::JobSignal;
use rastai_proto::worker::v1::ControlRequest;
use rastai_proto::worker::v1::worker_service_client::WorkerServiceClient;
use tokio::process::Command;

use crate::jobs::supervisor::WorkerLink;
use crate::worker_host_service::request_with_job_token;

const PROC_ROOT: &str = "/proc";
/// The kernel's id for this boot. Start ticks count from boot, so a pid reused after a reboot could carry the same
/// ticks; the boot id tells the two apart (layout 7.4).
const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
const STAT_FILE: &str = "stat";
/// The procps tool that signals a process group without `unsafe`.
const KILL_TOOL: &str = "kill";
/// In /proc/<pid>/stat, counted from 1: field 2 is the command name in parentheses (it may hold spaces and
/// parentheses itself, so fields are counted after its last `)`), field 3 the state, field 5 the process group,
/// field 22 the start time in clock ticks since boot.
const STAT_FIRST_FIELD_AFTER_NAME: usize = 3;
const STAT_STATE_FIELD: usize = 3;
const STAT_PROCESS_GROUP_FIELD: usize = 5;
const STAT_START_TICKS_FIELD: usize = 22;
/// The states of a process that has exited: a zombie (`Z`) its parent hasn't reaped yet, or one being torn down
/// (`X`, `x`). /proc still lists it, but it runs no more.
const EXITED_STATES: &[&str] = &["Z", "X", "x"];

/// Who a job's process is, as recorded at spawn: what `job.json` keeps for the pid-reuse check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub process_group_id: u32,
    pub start_ticks: u64,
    pub boot_id: String,
}

/// The operating-system signals the node sends to a job's process group (contracts 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSignal {
    /// CANCEL's fallback when the Control call fails: the worker's cancel flag.
    Interrupt,
    /// TERMINATE: preemption, exit 75.
    Terminate,
    /// KILL: the last resort.
    Kill,
}

/// The process group, start ticks and state of `/proc/<pid>/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatFields {
    pub process_group_id: u32,
    pub start_ticks: u64,
    /// A zombie or a process being torn down: it has exited, though /proc still lists it.
    pub has_exited: bool,
}

/// The identity of the process `pid`, or None when it is gone or `/proc` can't be read. A zombie still has one: a
/// worker that exits at once is still the node's child to wait on (spawn.rs).
pub fn read_process_identity(pid: u32) -> Option<ProcessIdentity> {
    let fields = read_stat_fields(pid)?;
    identity_from_stat(pid, &fields)
}

/// Whether `expected` is still the recorded process and has not exited. What a restarted node asks of a job it didn't
/// start (reattach.rs): such a job isn't its child, and once it exits it may stay a zombie until its parent reaps it.
pub fn is_running_process(expected: &ProcessIdentity) -> bool {
    let Some(fields) = read_stat_fields(expected.pid) else {
        return false;
    };
    !fields.has_exited && identity_from_stat(expected.pid, &fields).as_ref() == Some(expected)
}

fn read_stat_fields(pid: u32) -> Option<StatFields> {
    let stat_path = PathBuf::from(PROC_ROOT).join(pid.to_string()).join(STAT_FILE);
    let text = std::fs::read_to_string(stat_path).ok()?;
    parse_process_stat(&text)
}

fn identity_from_stat(pid: u32, fields: &StatFields) -> Option<ProcessIdentity> {
    Some(ProcessIdentity {
        pid,
        process_group_id: fields.process_group_id,
        start_ticks: fields.start_ticks,
        boot_id: read_boot_id()?,
    })
}

/// This boot's id, trimmed.
pub fn read_boot_id() -> Option<String> {
    let text = std::fs::read_to_string(BOOT_ID_PATH).ok()?;
    Some(text.trim().to_string())
}

/// Reads the two fields the identity needs from the text of `/proc/<pid>/stat`.
pub fn parse_process_stat(text: &str) -> Option<StatFields> {
    let after_name = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let field = |number: usize| fields.get(number - STAT_FIRST_FIELD_AFTER_NAME).copied();
    Some(StatFields {
        process_group_id: field(STAT_PROCESS_GROUP_FIELD)?.parse().ok()?,
        start_ticks: field(STAT_START_TICKS_FIELD)?.parse().ok()?,
        has_exited: EXITED_STATES.contains(&field(STAT_STATE_FIELD)?),
    })
}

/// Whether `expected.pid` is still the process the node recorded: same boot, same start ticks, same group.
pub fn is_same_process(expected: &ProcessIdentity) -> bool {
    read_process_identity(expected.pid).as_ref() == Some(expected)
}

/// Sends `signal` to the job's whole process group, after checking that its leader is still the recorded process.
/// The error says why nothing was sent, or what `kill` printed.
pub async fn signal_process_group(expected: &ProcessIdentity, signal: GroupSignal) -> Result<(), String> {
    if !is_same_process(expected) {
        return Err(format!(
            "process {} is gone or is no longer the job's process, so it wasn't signalled",
            expected.pid
        ));
    }
    let group_target = format!("-{}", expected.process_group_id);
    let output = Command::new(KILL_TOOL)
        .args(["-s", signal_name(signal), "--", &group_target])
        .output()
        .await
        .map_err(|error| format!("`{KILL_TOOL}` couldn't run: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let printed = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!("`{KILL_TOOL} -s {}` failed: {printed}", signal_name(signal)))
}

/// Calls `WorkerService.Control` with one of the six Control values. Ok holds the worker's own refusal, if any; the
/// error says why the call didn't go through.
pub async fn send_control(link: &WorkerLink, action: JobSignal) -> Result<Option<ProtoDiagnostic>, String> {
    let mut client = WorkerServiceClient::new(link.channel.clone());
    let message = ControlRequest { action: action as i32 };
    let request = request_with_job_token(message, &link.job_token)?;
    let response = client
        .control(request)
        .await
        .map_err(|status| format!("the Control call failed: {}", status.message()))?;
    Ok(response.into_inner().diagnostic)
}

/// The name `kill -s` takes.
pub fn signal_name(signal: GroupSignal) -> &'static str {
    match signal {
        GroupSignal::Interrupt => "INT",
        GroupSignal::Terminate => "TERM",
        GroupSignal::Kill => "KILL",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // A command name may hold spaces and parentheses; the fields are counted after the last `)`.
    #[test]
    fn stat_fields_are_read_after_the_command_name() {
        let text = "4242 (py (worker) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 3 0 987654 1000 200";
        let fields = parse_process_stat(text);
        assert_eq!(
            fields,
            Some(StatFields {
                process_group_id: 4242,
                start_ticks: 987654,
                has_exited: false,
            })
        );
        let zombie = text.replace(") S ", ") Z ");
        assert!(parse_process_stat(&zombie).unwrap().has_exited);
    }

    #[test]
    fn a_short_or_damaged_stat_reads_as_nothing() {
        assert_eq!(parse_process_stat("4242 (python) S 1 4242"), None);
        assert_eq!(parse_process_stat("no parenthesis here"), None);
        assert_eq!(
            parse_process_stat("1 (x) S 1 notanumber 1 0 -1 0 0 0 0 0 0 0 0 0 0 0 1 0 5"),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reads_as_itself_and_a_changed_record_does_not() {
        let own = read_process_identity(std::process::id()).expect("this process is in /proc");
        assert!(is_same_process(&own));
        let reused = ProcessIdentity {
            start_ticks: own.start_ticks + 1,
            ..own.clone()
        };
        let other_boot = ProcessIdentity {
            boot_id: "an-earlier-boot".to_string(),
            ..own
        };
        assert!(!is_same_process(&reused));
        assert!(!is_same_process(&other_boot));
        assert!(is_running_process(&read_process_identity(std::process::id()).unwrap()));
    }

    // An exited child that nobody has reaped yet is still in /proc with its identity, but it no longer runs.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_zombie_keeps_its_identity_but_is_not_running() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let identity = loop {
            let identity = read_process_identity(child.id()).unwrap();
            let fields = read_stat_fields(child.id()).unwrap();
            if fields.has_exited {
                break identity;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(is_same_process(&identity));
        assert!(!is_running_process(&identity));
        child.wait().unwrap();
        assert!(!is_same_process(&identity));
    }
}
