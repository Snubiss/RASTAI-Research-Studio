//! Why a job's process ended, and what that makes the job (layout: rust-node-gateway-proto.md section 7.4, "Exit
//! causes"; platform flow 5.5). `classify_exit` is a pure function of the facts the supervisor gathers when the
//! process exits: the exit code or signal, whether the node itself killed the group and why, whether the kernel's
//! out-of-memory counter rose, what this job wrote to the run's event log, and the tail of `stderr.log`.
//!
//! The worker's exit codes are `rastai_worker/exit_codes.py`'s (contracts 9.1). A worker that never registers has no
//! exit code to give: the node kills it after `WORKER_REGISTER_TIMEOUT_SECONDS` and the outcome is `RRS-J0002`.

use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::{Diagnostic, diagnostic, diagnostic_at};
use rastai_proto::common::v1::{EvalRunState, JobState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::node::v1::JobSignal;
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};

/// The worker's exit codes (contracts 9.1; python layout 7.2).
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_BAD_LAUNCH: i32 = 2;
pub const EXIT_CANCELLED: i32 = 3;
pub const EXIT_EVENT_LOG_UNWRITABLE: i32 = 4;
pub const EXIT_EVENT_LOG_LOCKED: i32 = 5;
pub const EXIT_PREEMPTED: i32 = 75;
/// The reason a job that honored SIGTERM ends with (contracts 9.1: `cancelled`, reason `preempted`).
pub const PREEMPTED_REASON: &str = "preempted";
/// A process killed by signal N reports exit code 128 + N in `JobInfo.exit_code`, as shells do, so a SIGKILL shows
/// as 137 (the JobInfo golden).
const SIGNAL_EXIT_CODE_BASE: i32 = 128;
const SIGKILL_NUMBER: i32 = 9;
/// Linux's signal numbers and names, for `RRS-J1001`'s message. Any other number is shown as `signal <n>`.
const SIGNAL_NAMES: &[(i32, &str)] = &[
    (1, "SIGHUP"),
    (2, "SIGINT"),
    (3, "SIGQUIT"),
    (4, "SIGILL"),
    (5, "SIGTRAP"),
    (6, "SIGABRT"),
    (7, "SIGBUS"),
    (8, "SIGFPE"),
    (9, "SIGKILL"),
    (10, "SIGUSR1"),
    (11, "SIGSEGV"),
    (12, "SIGUSR2"),
    (13, "SIGPIPE"),
    (14, "SIGALRM"),
    (15, "SIGTERM"),
    (24, "SIGXCPU"),
    (25, "SIGXFSZ"),
];
/// The cgroup v2 file whose `oom_kill` line counts the kernel's out-of-memory kills in a cgroup.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const MEMORY_EVENTS_FILE: &str = "memory.events";
const OOM_KILL_KEY: &str = "oom_kill";
/// `/proc/<pid>/cgroup` holds `0::<path>` on a cgroup v2 system.
const CGROUP_V2_PREFIX: &str = "0::";
/// The event types this file reads from the run's log (contracts 7.3).
const RUN_STATE_CHANGED: &str = "run.state_changed";
const DIAGNOSTIC_REPORTED: &str = "diagnostic.reported";
/// `JOB_SIGNAL_CANCEL` is named `CANCEL` in a message.
const JOB_SIGNAL_NAME_PREFIX: &str = "JOB_SIGNAL_";

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExit {
    Code(i32),
    Signal(i32),
}

/// Why the node killed a job's process group itself. The first cause is kept: a later one doesn't change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKill {
    /// No `Register` within `WORKER_REGISTER_TIMEOUT_SECONDS` (`RRS-J0002`).
    RegisterTimeout { timeout_seconds: u64 },
    /// Something registered for this job with another token (`RRS-J2002`): something else is on the socket.
    TokenRejected,
    /// `SignalJob KILL`.
    KillSignal,
    /// A CANCEL, STOP or TERMINATE that hadn't ended the job within `STOP_GRACE_SECONDS`.
    GraceExpired { signal: JobSignal, grace_seconds: u64 },
}

/// What this job wrote to the run's event log that its outcome depends on. Records of other jobs of the run (an
/// earlier attempt, or the running copy after exit 5) are never read as this job's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunLogFacts {
    /// A final `run.state_changed` to `completed`.
    pub completed: bool,
    pub last_diagnostic: Option<Diagnostic>,
}

/// The last lines of a job's `stderr.log`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StderrTail {
    /// Where the lines are, relative to the data folder (`runs/<run_id>/stderr.log`), for `Diagnostic.path`.
    pub path: String,
    pub lines: Vec<String>,
}

/// Everything `classify_exit` decides from.
#[derive(Debug, Clone, PartialEq)]
pub struct ExitFacts {
    pub job_id: String,
    pub exit: ProcessExit,
    /// The worker registered and answered `Hello`, so the job reached `running`.
    pub registered: bool,
    pub node_kill: Option<NodeKill>,
    /// The cgroup's `oom_kill` count rose between spawn and exit.
    pub oom_kill_rose: bool,
    /// Why the node refused the worker at `Register` without killing it (`RRS-J2003`: it was sent TERMINATE).
    pub start_refusal: Option<Diagnostic>,
    pub run_log: RunLogFacts,
    pub stderr: StderrTail,
    /// The run's event log, for `RRS-J3006`'s message.
    pub event_log_path: String,
    /// Free bytes where the run folder is, for `RRS-S3002`; None when they couldn't be read.
    pub free_bytes: Option<u64>,
}

/// What the job becomes.
#[derive(Debug, Clone, PartialEq)]
pub struct JobOutcome {
    pub state: JobState,
    /// Why, when the state alone doesn't say: `preempted` (contracts 9.1).
    pub reason: String,
    pub diagnostic: Option<Diagnostic>,
}

/// The job's state, reason and diagnostic for how its process ended (platform flow 5.5, one branch per row).
pub fn classify_exit(facts: &ExitFacts) -> JobOutcome {
    let outcome = match facts.exit {
        ProcessExit::Signal(number) => classify_signal(facts, number),
        ProcessExit::Code(code) => classify_code(facts, code),
    };
    refuse_success_before_register(facts, outcome)
}

/// `JobInfo.exit_code` for an exit: the code, or 128 plus the signal's number.
pub fn exit_code_for_info(exit: ProcessExit) -> i32 {
    match exit {
        ProcessExit::Code(code) => code,
        ProcessExit::Signal(number) => SIGNAL_EXIT_CODE_BASE + number,
    }
}

/// `SIGSEGV` for 11; `signal 40` for a number without a name here.
pub fn signal_number_name(number: i32) -> String {
    match SIGNAL_NAMES.iter().find(|(known, _)| *known == number) {
        Some((_, name)) => (*name).to_string(),
        None => format!("signal {number}"),
    }
}

/// How `std` reports an exit, as a `ProcessExit`. A status with neither a code nor a signal can't happen on Unix.
pub fn process_exit_from_status(status: std::process::ExitStatus) -> ProcessExit {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(number) = status.signal() {
            return ProcessExit::Signal(number);
        }
    }
    ProcessExit::Code(status.code().unwrap_or(-1))
}

/// Reads what job `job_id` wrote to the run's event log: whether its last `run.state_changed` is `completed`, and
/// its last `diagnostic.reported`. A missing log, or one that stops at a torn or corrupt frame, gives what was read
/// up to there: the outcome then falls to `RRS-J1004`, never to a guess.
pub fn read_run_log_facts(event_log_path: &Path, job_id: &str) -> RunLogFacts {
    let mut facts = RunLogFacts::default();
    let Ok(mut reader) = open_event_log(event_log_path) else {
        return facts;
    };
    while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
        if record.job_id != job_id {
            continue;
        }
        let payload = record.payload.and_then(|payload| payload.payload);
        match (record.event_type.as_str(), payload) {
            (RUN_STATE_CHANGED, Some(Payload::RunStateChanged(change))) => {
                facts.completed = is_completed(change.state);
            }
            (DIAGNOSTIC_REPORTED, Some(Payload::DiagnosticReported(reported))) => {
                if let Some(found) = reported.diagnostic {
                    facts.last_diagnostic = Some(rastai_core::wire::diagnostic_from_proto(&found));
                }
            }
            _ => {}
        }
    }
    facts
}

/// Where the kernel counts out-of-memory kills for a process's cgroup, and the count when the job started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OomWatch {
    pub memory_events_path: PathBuf,
    pub count_at_spawn: u64,
}

/// Reads the job's cgroup and its `oom_kill` count now. None where there is no cgroup v2 memory controller: the
/// node then never claims an out-of-memory kill (`RRS-J1001` names the signal instead).
pub fn start_oom_watch(pid: u32) -> Option<OomWatch> {
    let cgroup_text = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let cgroup_path = cgroup_text
        .lines()
        .find_map(|line| line.strip_prefix(CGROUP_V2_PREFIX))?;
    let memory_events_path = PathBuf::from(CGROUP_ROOT)
        .join(cgroup_path.trim_start_matches('/'))
        .join(MEMORY_EVENTS_FILE);
    let count_at_spawn = read_oom_kill_count(&memory_events_path)?;
    Some(OomWatch {
        memory_events_path,
        count_at_spawn,
    })
}

/// Whether the cgroup's `oom_kill` count is higher than when the job started.
pub fn oom_kill_rose(watch: &OomWatch) -> bool {
    match read_oom_kill_count(&watch.memory_events_path) {
        Some(count) => count > watch.count_at_spawn,
        None => false,
    }
}

/// The `oom_kill` value in the text of a `memory.events` file.
pub fn parse_oom_kill_count(text: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        if key == OOM_KILL_KEY {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn read_oom_kill_count(path: &Path) -> Option<u64> {
    parse_oom_kill_count(&std::fs::read_to_string(path).ok()?)
}

/// A training-style run ends `completed`; an eval run ends `passed` or `failed`, its verdict (contracts 9.6), and
/// either means the job did its work (`errored` and `cancelled` don't).
fn is_completed(state: Option<State>) -> bool {
    match state {
        Some(State::RunState(value)) => value == RunState::Completed as i32,
        Some(State::EvalRunState(value)) => {
            value == EvalRunState::Passed as i32 || value == EvalRunState::Failed as i32
        }
        None => false,
    }
}

fn classify_signal(facts: &ExitFacts, number: i32) -> JobOutcome {
    if let Some(kill) = facts.node_kill {
        return outcome_of_node_kill(facts, kill);
    }
    if number == SIGKILL_NUMBER && facts.oom_kill_rose {
        return failed(diagnostic_with_tail("RRS-J1002", &[("job_id", &facts.job_id)], facts));
    }
    let signal = signal_number_name(number);
    failed(diagnostic_with_tail(
        "RRS-J1001",
        &[("job_id", &facts.job_id), ("signal", &signal)],
        facts,
    ))
}

fn outcome_of_node_kill(facts: &ExitFacts, kill: NodeKill) -> JobOutcome {
    match kill {
        NodeKill::RegisterTimeout { timeout_seconds } => {
            let timeout = timeout_seconds.to_string();
            failed(diagnostic(
                "RRS-J0002",
                &[("job_id", &facts.job_id), ("timeout_seconds", &timeout)],
            ))
        }
        NodeKill::TokenRejected => failed(diagnostic("RRS-J2002", &[("job_id", &facts.job_id)])),
        NodeKill::KillSignal => cancelled_by_node(facts, "KILL was signalled"),
        NodeKill::GraceExpired { signal, grace_seconds } => {
            let reason = format!(
                "{} didn't end it within {grace_seconds} seconds",
                signal.as_str_name().trim_start_matches(JOB_SIGNAL_NAME_PREFIX)
            );
            cancelled_by_node(facts, &reason)
        }
    }
}

fn classify_code(facts: &ExitFacts, code: i32) -> JobOutcome {
    match code {
        EXIT_OK if facts.run_log.completed => JobOutcome {
            state: JobState::Succeeded,
            reason: String::new(),
            diagnostic: None,
        },
        EXIT_FAILED => match &facts.run_log.last_diagnostic {
            Some(logged) => failed(logged.clone()),
            None => no_final_record(facts, code),
        },
        EXIT_BAD_LAUNCH => {
            let reason = last_line_or(facts, "it reported a bad launch (exit 2)");
            failed(diagnostic_with_tail(
                "RRS-J2001",
                &[("job_id", &facts.job_id), ("reason", &reason)],
                facts,
            ))
        }
        EXIT_CANCELLED => cancelled(String::new(), None),
        EXIT_EVENT_LOG_UNWRITABLE => failed(disk_full_diagnostic(facts)),
        // Nothing is read from the run's log here: its frames are the running copy's (contracts 9.1).
        EXIT_EVENT_LOG_LOCKED => failed(diagnostic_with_tail(
            "RRS-J3006",
            &[("path", &facts.event_log_path)],
            facts,
        )),
        EXIT_PREEMPTED => cancelled(PREEMPTED_REASON.to_string(), facts.start_refusal.clone()),
        _ => no_final_record(facts, code),
    }
}

/// A job can only succeed once the node has met its worker (`starting` → `succeeded` isn't a transition, contracts
/// 9.1). A process that exits 0 before it registered isn't one the node can vouch for.
fn refuse_success_before_register(facts: &ExitFacts, outcome: JobOutcome) -> JobOutcome {
    if outcome.state == JobState::Succeeded && !facts.registered {
        return no_final_record(facts, exit_code_for_info(facts.exit));
    }
    outcome
}

fn no_final_record(facts: &ExitFacts, code: i32) -> JobOutcome {
    let exit_code = code.to_string();
    failed(diagnostic_with_tail(
        "RRS-J1004",
        &[("job_id", &facts.job_id), ("exit_code", &exit_code)],
        facts,
    ))
}

/// `RRS-S3002` with the stderr tail and the free space, the two things a user needs to see why the log failed.
fn disk_full_diagnostic(facts: &ExitFacts) -> Diagnostic {
    let last_line = last_line_or(facts, "the worker couldn't write the run's event log (exit 4)");
    let reason = match facts.free_bytes {
        Some(free_bytes) => format!("{last_line}; {free_bytes} bytes free where the run folder is"),
        None => last_line,
    };
    diagnostic_with_tail("RRS-S3002", &[("reason", &reason)], facts)
}

fn cancelled_by_node(facts: &ExitFacts, reason: &str) -> JobOutcome {
    let killed = diagnostic("RRS-J1003", &[("job_id", &facts.job_id), ("reason", reason)]);
    cancelled(String::new(), Some(killed))
}

/// The last non-empty line of stderr.log, or `fallback` when there is none.
fn last_line_or(facts: &ExitFacts, fallback: &str) -> String {
    let last = facts.stderr.lines.iter().rev().find(|line| !line.trim().is_empty());
    match last {
        Some(line) => line.trim().to_string(),
        None => fallback.to_string(),
    }
}

/// A diagnostic that points at the job's stderr.log, where the worker's own words are.
fn diagnostic_with_tail(code: &str, values: &[(&str, &str)], facts: &ExitFacts) -> Diagnostic {
    diagnostic_at(code, values, &facts.stderr.path)
}

fn failed(found: Diagnostic) -> JobOutcome {
    JobOutcome {
        state: JobState::Failed,
        reason: String::new(),
        diagnostic: Some(found),
    }
}

fn cancelled(reason: String, found: Option<Diagnostic>) -> JobOutcome {
    JobOutcome {
        state: JobState::Cancelled,
        reason,
        diagnostic: found,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const JOB: &str = "job_test";
    const STDERR_PATH: &str = "runs/run_test/stderr.log";

    fn facts(exit: ProcessExit) -> ExitFacts {
        ExitFacts {
            job_id: JOB.to_string(),
            exit,
            registered: true,
            node_kill: None,
            oom_kill_rose: false,
            start_refusal: None,
            run_log: RunLogFacts::default(),
            stderr: StderrTail {
                path: STDERR_PATH.to_string(),
                lines: vec![
                    "Traceback (most recent call last):".to_string(),
                    "OSError: boom".to_string(),
                ],
            },
            event_log_path: "runs/run_test/events.log".to_string(),
            free_bytes: Some(1024),
        }
    }

    fn code_of(outcome: &JobOutcome) -> &str {
        outcome.diagnostic.as_ref().map_or("", |found| found.code.as_str())
    }

    fn logged_diagnostic() -> Diagnostic {
        diagnostic("RRS-T3001", &[("run_id", "run_test"), ("count", "3")])
    }

    /// The exit codes of contracts 9.1 and what each gives: the facts, then the state, reason and code.
    fn exit_code_rows() -> Vec<(ExitFacts, JobState, &'static str, &'static str)> {
        let completed = RunLogFacts {
            completed: true,
            last_diagnostic: None,
        };
        let with_diagnostic = RunLogFacts {
            completed: false,
            last_diagnostic: Some(logged_diagnostic()),
        };
        let code = |code: i32| facts(ProcessExit::Code(code));
        vec![
            (with_run_log(code(0), completed.clone()), JobState::Succeeded, "", ""),
            (code(0), JobState::Failed, "", "RRS-J1004"),
            (
                with_run_log(code(1), with_diagnostic),
                JobState::Failed,
                "",
                "RRS-T3001",
            ),
            (code(1), JobState::Failed, "", "RRS-J1004"),
            (code(2), JobState::Failed, "", "RRS-J2001"),
            (code(3), JobState::Cancelled, "", ""),
            (code(4), JobState::Failed, "", "RRS-S3002"),
            (code(5), JobState::Failed, "", "RRS-J3006"),
            (code(75), JobState::Cancelled, "preempted", ""),
            (code(42), JobState::Failed, "", "RRS-J1004"),
            (
                ExitFacts {
                    registered: false,
                    ..with_run_log(code(0), completed)
                },
                JobState::Failed,
                "",
                "RRS-J1004",
            ),
        ]
    }

    /// The signal deaths: the kernel, another process, or the node itself and why.
    fn signal_rows() -> Vec<(ExitFacts, JobState, &'static str, &'static str)> {
        let killed = || facts(ProcessExit::Signal(9));
        let by_node = |kill: NodeKill| ExitFacts {
            node_kill: Some(kill),
            ..killed()
        };
        let grace = NodeKill::GraceExpired {
            signal: JobSignal::Cancel,
            grace_seconds: 120,
        };
        let out_of_memory = ExitFacts {
            oom_kill_rose: true,
            ..killed()
        };
        vec![
            (killed(), JobState::Failed, "", "RRS-J1001"),
            (facts(ProcessExit::Signal(11)), JobState::Failed, "", "RRS-J1001"),
            (out_of_memory, JobState::Failed, "", "RRS-J1002"),
            (by_node(NodeKill::KillSignal), JobState::Cancelled, "", "RRS-J1003"),
            (by_node(grace), JobState::Cancelled, "", "RRS-J1003"),
            (
                by_node(NodeKill::RegisterTimeout { timeout_seconds: 60 }),
                JobState::Failed,
                "",
                "RRS-J0002",
            ),
            (by_node(NodeKill::TokenRejected), JobState::Failed, "", "RRS-J2002"),
        ]
    }

    fn with_run_log(exit_facts: ExitFacts, run_log: RunLogFacts) -> ExitFacts {
        ExitFacts { run_log, ..exit_facts }
    }

    /// One row per branch of platform flow 5.5.
    #[test]
    fn every_exit_maps_to_its_state_and_code() {
        let mut rows = exit_code_rows();
        rows.extend(signal_rows());
        for (row, (exit_facts, state, reason, code)) in rows.iter().enumerate() {
            let outcome = classify_exit(exit_facts);
            assert_eq!(outcome.state, *state, "row {row}: {exit_facts:?}");
            assert_eq!(outcome.reason, *reason, "row {row}");
            assert_eq!(code_of(&outcome), *code, "row {row}");
        }
    }

    // Exit 4 shows the worker's last words and the free space; exit 5 names the log and points at stderr.log.
    #[test]
    fn the_disk_full_and_locked_log_diagnostics_carry_their_details() {
        let disk_full = classify_exit(&facts(ProcessExit::Code(4))).diagnostic.unwrap();
        assert!(disk_full.message.contains("OSError: boom"), "{}", disk_full.message);
        assert!(disk_full.message.contains("1024 bytes free"), "{}", disk_full.message);
        assert_eq!(disk_full.path, STDERR_PATH);
        let locked = classify_exit(&facts(ProcessExit::Code(5))).diagnostic.unwrap();
        assert!(
            locked.message.contains("runs/run_test/events.log"),
            "{}",
            locked.message
        );
        assert_eq!(locked.path, STDERR_PATH);
    }

    // Exit 5 never reads the log, so even a completed record of this job's id can't turn it into a success.
    #[test]
    fn exit_five_ignores_the_run_log() {
        let exit_facts = ExitFacts {
            run_log: RunLogFacts {
                completed: true,
                last_diagnostic: Some(logged_diagnostic()),
            },
            ..facts(ProcessExit::Code(5))
        };
        assert_eq!(code_of(&classify_exit(&exit_facts)), "RRS-J3006");
    }

    // A worker the node refused for its protocol version was sent TERMINATE; its exit 75 keeps that reason.
    #[test]
    fn a_refused_protocol_keeps_its_diagnostic_after_preemption() {
        let refusal = diagnostic("RRS-J2003", &[("job_id", JOB), ("version", "2")]);
        let exit_facts = ExitFacts {
            start_refusal: Some(refusal),
            ..facts(ProcessExit::Code(75))
        };
        let outcome = classify_exit(&exit_facts);
        assert_eq!(outcome.state, JobState::Cancelled);
        assert_eq!(outcome.reason, PREEMPTED_REASON);
        assert_eq!(code_of(&outcome), "RRS-J2003");
    }

    #[test]
    fn signals_are_named_and_shown_as_shell_exit_codes() {
        assert_eq!(signal_number_name(9), "SIGKILL");
        assert_eq!(signal_number_name(11), "SIGSEGV");
        assert_eq!(signal_number_name(40), "signal 40");
        assert_eq!(exit_code_for_info(ProcessExit::Signal(9)), 137);
        assert_eq!(exit_code_for_info(ProcessExit::Code(75)), 75);
        let killed = classify_exit(&facts(ProcessExit::Signal(9))).diagnostic.unwrap();
        assert!(killed.message.contains("SIGKILL"), "{}", killed.message);
    }

    #[test]
    fn the_oom_kill_count_is_read_from_memory_events() {
        let text = "low 0\nhigh 3\nmax 7\noom 2\noom_kill 2\noom_group_kill 0\n";
        assert_eq!(parse_oom_kill_count(text), Some(2));
        assert_eq!(parse_oom_kill_count("low 0\n"), None);
    }
}
