//! Names and transition tables for the state enums of `rastai.common.v1` (contracts 9). The enums themselves are
//! rastai-proto's (one owner); this file adds, for each machine, four plain helpers: the state's name as stored in
//! `rastai.db` and shown in logs (`waiting_for_lease`), parsing that name back, whether a transition is allowed,
//! and whether a state is terminal.
//!
//! Each table is the union of contracts 9 and the chart in the flow doc that draws the machine, which contracts 9
//! defers to ("names only; the flow docs draw them"). Where a flow adds a transition, the comment above the table
//! says where. Two readings worth knowing:
//! - "Any state → X" covers every state except X itself and the terminal ones.
//! - Contracts 9.2's "unreachable → connecting → connected" is shorthand for connecting → handshaking → connected:
//!   every connection is handshaken (platform flow 2.2, remote flow C2).
//!
//! `UNSPECIFIED` is the proto default, not a state anything is in: it has the name `unspecified` for logs, isn't
//! parsed, and takes part in no transition.

use rastai_proto::common::v1::{
    EvalRunState, JobState, LeaseState, ModelState, NodeState, PluginState, RunState, SandboxState,
};
use rastai_proto::node::v1::JobKind;

const UNSPECIFIED_NAME: &str = "unspecified";
/// The proto names of job kinds start with this; contracts 9.1 names them without it, in lower case.
const JOB_KIND_NAME_PREFIX: &str = "JOB_KIND_";

// ---- Job (contracts 9.1, platform flow 5.2) ----

const JOB_STATES: &[JobState] = &[
    JobState::Queued,
    JobState::Starting,
    JobState::Running,
    JobState::Paused,
    JobState::Stopping,
    JobState::Succeeded,
    JobState::Failed,
    JobState::Cancelled,
    JobState::Lost,
];

const JOB_TERMINAL_STATES: &[JobState] = &[JobState::Succeeded, JobState::Failed, JobState::Cancelled];

const JOB_TRANSITIONS: &[(JobState, JobState)] = &[
    (JobState::Queued, JobState::Starting),
    (JobState::Queued, JobState::Cancelled),
    (JobState::Queued, JobState::Failed),
    (JobState::Starting, JobState::Running),
    (JobState::Starting, JobState::Failed),
    (JobState::Running, JobState::Paused),
    (JobState::Paused, JobState::Running),
    (JobState::Running, JobState::Stopping),
    (JobState::Paused, JobState::Stopping),
    (JobState::Running, JobState::Succeeded),
    (JobState::Stopping, JobState::Succeeded),
    (JobState::Running, JobState::Cancelled),
    (JobState::Stopping, JobState::Cancelled),
    // "Any live state → cancelled": a SIGKILL the node sent also reaches a job that is starting or paused.
    (JobState::Starting, JobState::Cancelled),
    (JobState::Paused, JobState::Cancelled),
    (JobState::Running, JobState::Failed),
    (JobState::Stopping, JobState::Failed),
    (JobState::Running, JobState::Lost),
    (JobState::Lost, JobState::Running),
    (JobState::Lost, JobState::Succeeded),
    (JobState::Lost, JobState::Failed),
    (JobState::Lost, JobState::Cancelled),
];

pub fn job_state_name(state: JobState) -> &'static str {
    match state {
        JobState::Unspecified => UNSPECIFIED_NAME,
        JobState::Queued => "queued",
        JobState::Starting => "starting",
        JobState::Running => "running",
        JobState::Paused => "paused",
        JobState::Stopping => "stopping",
        JobState::Succeeded => "succeeded",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Lost => "lost",
    }
}

pub fn parse_job_state(text: &str) -> Option<JobState> {
    JOB_STATES.iter().copied().find(|state| job_state_name(*state) == text)
}

pub fn job_transition_allowed(from: JobState, to: JobState) -> bool {
    JOB_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_job_state(state: JobState) -> bool {
    JOB_TERMINAL_STATES.contains(&state)
}

/// A job kind's name as `jobs.kind` holds it (contracts 9.1): the proto name without `JOB_KIND_`, in lower case
/// (`train`, `doctor_selftest`). A number this build doesn't know is written as the number, so it is never lost.
pub fn job_kind_name(kind: i32) -> String {
    match JobKind::try_from(kind) {
        Ok(kind) => kind
            .as_str_name()
            .trim_start_matches(JOB_KIND_NAME_PREFIX)
            .to_ascii_lowercase(),
        Err(_) => kind.to_string(),
    }
}

// ---- Training run (contracts 9.5, training flow 4.2 and the chain chart of 6) ----

const RUN_STATES: &[RunState] = &[
    RunState::Created,
    RunState::Preparing,
    RunState::Running,
    RunState::Checkpointing,
    RunState::Paused,
    RunState::Preempted,
    RunState::Resuming,
    RunState::Verifying,
    RunState::Completed,
    RunState::Failed,
    RunState::Cancelled,
];

// `failed` and `cancelled` end a run but aren't terminal: `train.resume` reopens either from a complete checkpoint.
// Only `completed` always is (contracts 9.5).
const RUN_TERMINAL_STATES: &[RunState] = &[RunState::Completed];

const RUN_TRANSITIONS: &[(RunState, RunState)] = &[
    (RunState::Created, RunState::Preparing),
    (RunState::Preparing, RunState::Running),
    (RunState::Preparing, RunState::Failed),
    (RunState::Running, RunState::Checkpointing),
    (RunState::Checkpointing, RunState::Running),
    (RunState::Running, RunState::Paused),
    (RunState::Paused, RunState::Running),
    (RunState::Running, RunState::Preempted),
    (RunState::Preempted, RunState::Resuming),
    (RunState::Resuming, RunState::Running),
    (RunState::Resuming, RunState::Failed),
    (RunState::Failed, RunState::Resuming),
    (RunState::Cancelled, RunState::Resuming),
    (RunState::Running, RunState::Verifying),
    (RunState::Verifying, RunState::Completed),
    (RunState::Running, RunState::Completed),
    (RunState::Running, RunState::Failed),
    (RunState::Running, RunState::Cancelled),
    // Chain runs (training flow 6): a verify step can fail, and train.stop cancels the chain in any phase.
    (RunState::Verifying, RunState::Failed),
    (RunState::Preparing, RunState::Cancelled),
    (RunState::Verifying, RunState::Cancelled),
    // A job that ends before the worker's first record (contracts 9.5): the daemon records a spawn error, a bad
    // launch or a KILL as failed or cancelled, `job.cancel` of a queued job as cancelled, and the worker's edge
    // records a CANCEL or SIGTERM that came before the entry ran.
    (RunState::Created, RunState::Failed),
    (RunState::Created, RunState::Cancelled),
    (RunState::Created, RunState::Preempted),
    // Every way a job ends reaches a run that is checkpointing (contracts 9.1: KILL in any live state).
    (RunState::Checkpointing, RunState::Paused),
    (RunState::Checkpointing, RunState::Preempted),
    (RunState::Checkpointing, RunState::Completed),
    (RunState::Checkpointing, RunState::Failed),
    (RunState::Checkpointing, RunState::Cancelled),
    // A paused worker still answers Control and signals (contracts 9.1), so a paused run ends like a running one.
    (RunState::Paused, RunState::Completed),
    (RunState::Paused, RunState::Preempted),
    (RunState::Paused, RunState::Failed),
    (RunState::Paused, RunState::Cancelled),
    // The job of a resume can be cancelled or preempted before it restored the checkpoint.
    (RunState::Resuming, RunState::Cancelled),
    (RunState::Resuming, RunState::Preempted),
];

pub fn run_state_name(state: RunState) -> &'static str {
    match state {
        RunState::Unspecified => UNSPECIFIED_NAME,
        RunState::Created => "created",
        RunState::Preparing => "preparing",
        RunState::Running => "running",
        RunState::Checkpointing => "checkpointing",
        RunState::Paused => "paused",
        RunState::Preempted => "preempted",
        RunState::Resuming => "resuming",
        RunState::Verifying => "verifying",
        RunState::Completed => "completed",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
    }
}

pub fn parse_run_state(text: &str) -> Option<RunState> {
    RUN_STATES.iter().copied().find(|state| run_state_name(*state) == text)
}

pub fn run_transition_allowed(from: RunState, to: RunState) -> bool {
    RUN_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_run_state(state: RunState) -> bool {
    RUN_TERMINAL_STATES.contains(&state)
}

// ---- Model load (contracts 9.3, inference flow 4.1) ----

const MODEL_STATES: &[ModelState] = &[
    ModelState::Unloaded,
    ModelState::Downloading,
    ModelState::WaitingForLease,
    ModelState::Loading,
    ModelState::Warming,
    ModelState::Ready,
    ModelState::Evicting,
    ModelState::Failed,
];

// A model always goes back to `unloaded`; none of its states is terminal.
const MODEL_TERMINAL_STATES: &[ModelState] = &[];

const MODEL_TRANSITIONS: &[(ModelState, ModelState)] = &[
    (ModelState::Unloaded, ModelState::Downloading),
    // The inference flow: weights already in the HF cache skip the download.
    (ModelState::Unloaded, ModelState::WaitingForLease),
    (ModelState::Downloading, ModelState::WaitingForLease),
    (ModelState::WaitingForLease, ModelState::Loading),
    (ModelState::Loading, ModelState::Warming),
    (ModelState::Warming, ModelState::Ready),
    (ModelState::Ready, ModelState::Evicting),
    (ModelState::Evicting, ModelState::Unloaded),
    // "Any state → failed".
    (ModelState::Unloaded, ModelState::Failed),
    (ModelState::Downloading, ModelState::Failed),
    (ModelState::WaitingForLease, ModelState::Failed),
    (ModelState::Loading, ModelState::Failed),
    (ModelState::Warming, ModelState::Failed),
    (ModelState::Ready, ModelState::Failed),
    (ModelState::Evicting, ModelState::Failed),
    (ModelState::Failed, ModelState::Unloaded),
];

pub fn model_state_name(state: ModelState) -> &'static str {
    match state {
        ModelState::Unspecified => UNSPECIFIED_NAME,
        ModelState::Unloaded => "unloaded",
        ModelState::Downloading => "downloading",
        ModelState::WaitingForLease => "waiting_for_lease",
        ModelState::Loading => "loading",
        ModelState::Warming => "warming",
        ModelState::Ready => "ready",
        ModelState::Evicting => "evicting",
        ModelState::Failed => "failed",
    }
}

pub fn parse_model_state(text: &str) -> Option<ModelState> {
    MODEL_STATES
        .iter()
        .copied()
        .find(|state| model_state_name(*state) == text)
}

pub fn model_transition_allowed(from: ModelState, to: ModelState) -> bool {
    MODEL_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_model_state(state: ModelState) -> bool {
    MODEL_TERMINAL_STATES.contains(&state)
}

// ---- VRAM lease (contracts 9.4) ----

const LEASE_STATES: &[LeaseState] = &[
    LeaseState::Requested,
    LeaseState::Granted,
    LeaseState::ReclaimRequested,
    LeaseState::Released,
    LeaseState::Denied,
];

const LEASE_TERMINAL_STATES: &[LeaseState] = &[LeaseState::Released, LeaseState::Denied];

const LEASE_TRANSITIONS: &[(LeaseState, LeaseState)] = &[
    (LeaseState::Requested, LeaseState::Granted),
    (LeaseState::Requested, LeaseState::Denied),
    (LeaseState::Granted, LeaseState::ReclaimRequested),
    (LeaseState::ReclaimRequested, LeaseState::Released),
    (LeaseState::Granted, LeaseState::Released),
];

pub fn lease_state_name(state: LeaseState) -> &'static str {
    match state {
        LeaseState::Unspecified => UNSPECIFIED_NAME,
        LeaseState::Requested => "requested",
        LeaseState::Granted => "granted",
        LeaseState::ReclaimRequested => "reclaim_requested",
        LeaseState::Released => "released",
        LeaseState::Denied => "denied",
    }
}

pub fn parse_lease_state(text: &str) -> Option<LeaseState> {
    LEASE_STATES
        .iter()
        .copied()
        .find(|state| lease_state_name(*state) == text)
}

pub fn lease_transition_allowed(from: LeaseState, to: LeaseState) -> bool {
    LEASE_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_lease_state(state: LeaseState) -> bool {
    LEASE_TERMINAL_STATES.contains(&state)
}

// ---- Node connection (contracts 9.2, platform flow 2.2, remote flow C2) ----

const NODE_STATES: &[NodeState] = &[
    NodeState::Unpaired,
    NodeState::Pairing,
    NodeState::Connecting,
    NodeState::Handshaking,
    NodeState::Connected,
    NodeState::Unreachable,
    NodeState::Incompatible,
];

// A node can always be paired again; none of its states is terminal.
const NODE_TERMINAL_STATES: &[NodeState] = &[];

const NODE_TRANSITIONS: &[(NodeState, NodeState)] = &[
    (NodeState::Unpaired, NodeState::Pairing),
    (NodeState::Pairing, NodeState::Connecting),
    (NodeState::Pairing, NodeState::Unpaired),
    (NodeState::Connecting, NodeState::Handshaking),
    (NodeState::Handshaking, NodeState::Connected),
    (NodeState::Handshaking, NodeState::Incompatible),
    // The flows: an identity check that fails retries with backoff.
    (NodeState::Handshaking, NodeState::Connecting),
    (NodeState::Connected, NodeState::Unreachable),
    (NodeState::Unreachable, NodeState::Connecting),
    (NodeState::Incompatible, NodeState::Connecting),
    // "Any state → unpaired" on node.unpair.
    (NodeState::Connecting, NodeState::Unpaired),
    (NodeState::Handshaking, NodeState::Unpaired),
    (NodeState::Connected, NodeState::Unpaired),
    (NodeState::Unreachable, NodeState::Unpaired),
    (NodeState::Incompatible, NodeState::Unpaired),
];

pub fn node_state_name(state: NodeState) -> &'static str {
    match state {
        NodeState::Unspecified => UNSPECIFIED_NAME,
        NodeState::Unpaired => "unpaired",
        NodeState::Pairing => "pairing",
        NodeState::Connecting => "connecting",
        NodeState::Handshaking => "handshaking",
        NodeState::Connected => "connected",
        NodeState::Unreachable => "unreachable",
        NodeState::Incompatible => "incompatible",
    }
}

pub fn parse_node_state(text: &str) -> Option<NodeState> {
    NODE_STATES
        .iter()
        .copied()
        .find(|state| node_state_name(*state) == text)
}

pub fn node_transition_allowed(from: NodeState, to: NodeState) -> bool {
    NODE_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_node_state(state: NodeState) -> bool {
    NODE_TERMINAL_STATES.contains(&state)
}

// ---- Sandbox (contracts 9.8, remote flow B4) ----

const SANDBOX_STATES: &[SandboxState] = &[
    SandboxState::Requested,
    SandboxState::Pulling,
    SandboxState::Creating,
    SandboxState::Running,
    SandboxState::Paused,
    SandboxState::Stopping,
    SandboxState::Removed,
    SandboxState::Failed,
    SandboxState::Orphaned,
];

const SANDBOX_TERMINAL_STATES: &[SandboxState] = &[SandboxState::Removed];

const SANDBOX_TRANSITIONS: &[(SandboxState, SandboxState)] = &[
    (SandboxState::Requested, SandboxState::Pulling),
    (SandboxState::Pulling, SandboxState::Creating),
    (SandboxState::Creating, SandboxState::Running),
    (SandboxState::Running, SandboxState::Paused),
    (SandboxState::Paused, SandboxState::Running),
    (SandboxState::Running, SandboxState::Stopping),
    // The flow: a hold timeout or a stopped run stops a paused sandbox.
    (SandboxState::Paused, SandboxState::Stopping),
    (SandboxState::Stopping, SandboxState::Removed),
    // "Any → failed".
    (SandboxState::Requested, SandboxState::Failed),
    (SandboxState::Pulling, SandboxState::Failed),
    (SandboxState::Creating, SandboxState::Failed),
    (SandboxState::Running, SandboxState::Failed),
    (SandboxState::Paused, SandboxState::Failed),
    (SandboxState::Stopping, SandboxState::Failed),
    (SandboxState::Orphaned, SandboxState::Failed),
    (SandboxState::Failed, SandboxState::Removed),
    // The flow names the states a container with no live owner is found in.
    (SandboxState::Creating, SandboxState::Orphaned),
    (SandboxState::Running, SandboxState::Orphaned),
    (SandboxState::Paused, SandboxState::Orphaned),
    (SandboxState::Orphaned, SandboxState::Removed),
];

pub fn sandbox_state_name(state: SandboxState) -> &'static str {
    match state {
        SandboxState::Unspecified => UNSPECIFIED_NAME,
        SandboxState::Requested => "requested",
        SandboxState::Pulling => "pulling",
        SandboxState::Creating => "creating",
        SandboxState::Running => "running",
        SandboxState::Paused => "paused",
        SandboxState::Stopping => "stopping",
        SandboxState::Removed => "removed",
        SandboxState::Failed => "failed",
        SandboxState::Orphaned => "orphaned",
    }
}

pub fn parse_sandbox_state(text: &str) -> Option<SandboxState> {
    SANDBOX_STATES
        .iter()
        .copied()
        .find(|state| sandbox_state_name(*state) == text)
}

pub fn sandbox_transition_allowed(from: SandboxState, to: SandboxState) -> bool {
    SANDBOX_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_sandbox_state(state: SandboxState) -> bool {
    SANDBOX_TERMINAL_STATES.contains(&state)
}

// ---- Plugin install (contracts 9.7, platform flow 7.2) ----

const PLUGIN_STATES: &[PluginState] = &[
    PluginState::Discovered,
    PluginState::Resolving,
    PluginState::AwaitingTrust,
    PluginState::Installing,
    PluginState::Installed,
    PluginState::Enabled,
    PluginState::Disabled,
    PluginState::Incompatible,
    PluginState::Failed,
    PluginState::Uninstalled,
];

const PLUGIN_TERMINAL_STATES: &[PluginState] = &[PluginState::Uninstalled];

const PLUGIN_TRANSITIONS: &[(PluginState, PluginState)] = &[
    (PluginState::Discovered, PluginState::Resolving),
    (PluginState::Resolving, PluginState::AwaitingTrust),
    (PluginState::AwaitingTrust, PluginState::Installing),
    (PluginState::Installing, PluginState::Installed),
    (PluginState::Installed, PluginState::Enabled),
    (PluginState::Resolving, PluginState::Incompatible),
    (PluginState::Enabled, PluginState::Incompatible),
    (PluginState::Incompatible, PluginState::Resolving),
    (PluginState::Enabled, PluginState::Disabled),
    (PluginState::Disabled, PluginState::Enabled),
    (PluginState::AwaitingTrust, PluginState::Uninstalled),
    (PluginState::Installed, PluginState::Uninstalled),
    (PluginState::Disabled, PluginState::Uninstalled),
    // An update that adds permissions asks again.
    (PluginState::Enabled, PluginState::AwaitingTrust),
    // The platform flow: a failed first load disables, and a failed or incompatible plugin can be uninstalled.
    (PluginState::Installed, PluginState::Disabled),
    (PluginState::Failed, PluginState::Uninstalled),
    (PluginState::Incompatible, PluginState::Uninstalled),
    // Every start checks an installed plugin again (platform flow 7.3): its engines, capabilities or manifest may no
    // longer fit, and permissions it was never granted ask again.
    (PluginState::Installed, PluginState::Incompatible),
    (PluginState::Installed, PluginState::AwaitingTrust),
    // "Any → failed".
    (PluginState::Discovered, PluginState::Failed),
    (PluginState::Resolving, PluginState::Failed),
    (PluginState::AwaitingTrust, PluginState::Failed),
    (PluginState::Installing, PluginState::Failed),
    (PluginState::Installed, PluginState::Failed),
    (PluginState::Enabled, PluginState::Failed),
    (PluginState::Disabled, PluginState::Failed),
    (PluginState::Incompatible, PluginState::Failed),
];

pub fn plugin_state_name(state: PluginState) -> &'static str {
    match state {
        PluginState::Unspecified => UNSPECIFIED_NAME,
        PluginState::Discovered => "discovered",
        PluginState::Resolving => "resolving",
        PluginState::AwaitingTrust => "awaiting_trust",
        PluginState::Installing => "installing",
        PluginState::Installed => "installed",
        PluginState::Enabled => "enabled",
        PluginState::Disabled => "disabled",
        PluginState::Incompatible => "incompatible",
        PluginState::Failed => "failed",
        PluginState::Uninstalled => "uninstalled",
    }
}

pub fn parse_plugin_state(text: &str) -> Option<PluginState> {
    PLUGIN_STATES
        .iter()
        .copied()
        .find(|state| plugin_state_name(*state) == text)
}

pub fn plugin_transition_allowed(from: PluginState, to: PluginState) -> bool {
    PLUGIN_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_plugin_state(state: PluginState) -> bool {
    PLUGIN_TERMINAL_STATES.contains(&state)
}

// ---- Eval run (contracts 9.6, training flow 9) ----

const EVAL_RUN_STATES: &[EvalRunState] = &[
    EvalRunState::Created,
    EvalRunState::Resolving,
    EvalRunState::Generating,
    EvalRunState::Judging,
    EvalRunState::Scoring,
    EvalRunState::Passed,
    EvalRunState::Failed,
    EvalRunState::Errored,
    EvalRunState::Cancelled,
];

const EVAL_RUN_TERMINAL_STATES: &[EvalRunState] = &[
    EvalRunState::Passed,
    EvalRunState::Failed,
    EvalRunState::Errored,
    EvalRunState::Cancelled,
];

const EVAL_RUN_TRANSITIONS: &[(EvalRunState, EvalRunState)] = &[
    (EvalRunState::Created, EvalRunState::Resolving),
    (EvalRunState::Resolving, EvalRunState::Generating),
    (EvalRunState::Generating, EvalRunState::Judging),
    // Judging is skipped when the suite has no judge tests.
    (EvalRunState::Generating, EvalRunState::Scoring),
    (EvalRunState::Judging, EvalRunState::Scoring),
    (EvalRunState::Scoring, EvalRunState::Passed),
    (EvalRunState::Scoring, EvalRunState::Failed),
    // "Any → errored or cancelled".
    (EvalRunState::Created, EvalRunState::Errored),
    (EvalRunState::Resolving, EvalRunState::Errored),
    (EvalRunState::Generating, EvalRunState::Errored),
    (EvalRunState::Judging, EvalRunState::Errored),
    (EvalRunState::Scoring, EvalRunState::Errored),
    (EvalRunState::Created, EvalRunState::Cancelled),
    (EvalRunState::Resolving, EvalRunState::Cancelled),
    (EvalRunState::Generating, EvalRunState::Cancelled),
    (EvalRunState::Judging, EvalRunState::Cancelled),
    (EvalRunState::Scoring, EvalRunState::Cancelled),
];

pub fn eval_run_state_name(state: EvalRunState) -> &'static str {
    match state {
        EvalRunState::Unspecified => UNSPECIFIED_NAME,
        EvalRunState::Created => "created",
        EvalRunState::Resolving => "resolving",
        EvalRunState::Generating => "generating",
        EvalRunState::Judging => "judging",
        EvalRunState::Scoring => "scoring",
        EvalRunState::Passed => "passed",
        EvalRunState::Failed => "failed",
        EvalRunState::Errored => "errored",
        EvalRunState::Cancelled => "cancelled",
    }
}

pub fn parse_eval_run_state(text: &str) -> Option<EvalRunState> {
    EVAL_RUN_STATES
        .iter()
        .copied()
        .find(|state| eval_run_state_name(*state) == text)
}

pub fn eval_run_transition_allowed(from: EvalRunState, to: EvalRunState) -> bool {
    EVAL_RUN_TRANSITIONS.contains(&(from, to))
}

pub fn is_terminal_eval_run_state(state: EvalRunState) -> bool {
    EVAL_RUN_TERMINAL_STATES.contains(&state)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_job_kind_is_named_without_its_proto_prefix() {
        assert_eq!(job_kind_name(JobKind::Train as i32), "train");
        assert_eq!(job_kind_name(JobKind::DoctorSelftest as i32), "doctor_selftest");
        assert_eq!(job_kind_name(999), "999");
    }

    // Each machine's transitions in contracts 9's words, "from -> to", read with the parse functions, so a wrong
    // name or a missing table row fails here. The flow-doc additions are listed after a blank line.
    const JOB_CONTRACT: &[&str] = &[
        "queued -> starting",
        "queued -> cancelled",
        "queued -> failed",
        "starting -> running",
        "starting -> failed",
        "running -> paused",
        "paused -> running",
        "running -> stopping",
        "paused -> stopping",
        "running -> succeeded",
        "stopping -> succeeded",
        "running -> cancelled",
        "stopping -> cancelled",
        "starting -> cancelled",
        "paused -> cancelled",
        "running -> failed",
        "stopping -> failed",
        "running -> lost",
        "lost -> running",
        "lost -> succeeded",
        "lost -> failed",
        "lost -> cancelled",
    ];
    const RUN_CONTRACT: &[&str] = &[
        "created -> preparing",
        "preparing -> running",
        "running -> checkpointing",
        "checkpointing -> running",
        "preparing -> failed",
        "running -> paused",
        "paused -> running",
        "running -> preempted",
        "preempted -> resuming",
        "resuming -> running",
        "resuming -> failed",
        "failed -> resuming",
        "cancelled -> resuming",
        "running -> verifying",
        "verifying -> completed",
        "running -> completed",
        "running -> failed",
        "running -> cancelled",
        "created -> failed",
        "created -> cancelled",
        "created -> preempted",
        "checkpointing -> paused",
        "checkpointing -> preempted",
        "checkpointing -> completed",
        "checkpointing -> failed",
        "checkpointing -> cancelled",
        "paused -> completed",
        "paused -> preempted",
        "paused -> failed",
        "paused -> cancelled",
        "resuming -> cancelled",
        "resuming -> preempted",
        //
        "verifying -> failed",
        "preparing -> cancelled",
        "verifying -> cancelled",
    ];
    const MODEL_CONTRACT: &[&str] = &[
        "unloaded -> downloading",
        "downloading -> waiting_for_lease",
        "waiting_for_lease -> loading",
        "loading -> warming",
        "warming -> ready",
        "ready -> evicting",
        "evicting -> unloaded",
        "unloaded -> failed",
        "downloading -> failed",
        "waiting_for_lease -> failed",
        "loading -> failed",
        "warming -> failed",
        "ready -> failed",
        "evicting -> failed",
        "failed -> unloaded",
        //
        "unloaded -> waiting_for_lease",
    ];
    const LEASE_CONTRACT: &[&str] = &[
        "requested -> granted",
        "requested -> denied",
        "granted -> reclaim_requested",
        "reclaim_requested -> released",
        "granted -> released",
    ];
    const NODE_CONTRACT: &[&str] = &[
        "unpaired -> pairing",
        "pairing -> connecting",
        "connecting -> handshaking",
        "handshaking -> connected",
        "connected -> unreachable",
        "unreachable -> connecting",
        "handshaking -> incompatible",
        "incompatible -> connecting",
        "pairing -> unpaired",
        "connecting -> unpaired",
        "handshaking -> unpaired",
        "connected -> unpaired",
        "unreachable -> unpaired",
        "incompatible -> unpaired",
        //
        "handshaking -> connecting",
    ];
    const SANDBOX_CONTRACT: &[&str] = &[
        "requested -> pulling",
        "pulling -> creating",
        "creating -> running",
        "running -> paused",
        "paused -> running",
        "running -> stopping",
        "stopping -> removed",
        "requested -> failed",
        "pulling -> failed",
        "creating -> failed",
        "running -> failed",
        "paused -> failed",
        "stopping -> failed",
        "orphaned -> failed",
        "failed -> removed",
        "orphaned -> removed",
        //
        "paused -> stopping",
        "creating -> orphaned",
        "running -> orphaned",
        "paused -> orphaned",
    ];
    const PLUGIN_CONTRACT: &[&str] = &[
        "discovered -> resolving",
        "resolving -> awaiting_trust",
        "awaiting_trust -> installing",
        "installing -> installed",
        "installed -> enabled",
        "resolving -> incompatible",
        "enabled -> incompatible",
        "incompatible -> resolving",
        "enabled -> disabled",
        "disabled -> enabled",
        "awaiting_trust -> uninstalled",
        "installed -> uninstalled",
        "disabled -> uninstalled",
        "enabled -> awaiting_trust",
        "discovered -> failed",
        "resolving -> failed",
        "awaiting_trust -> failed",
        "installing -> failed",
        "installed -> failed",
        "enabled -> failed",
        "disabled -> failed",
        "incompatible -> failed",
        //
        "installed -> disabled",
        "failed -> uninstalled",
        "incompatible -> uninstalled",
        "installed -> incompatible",
        "installed -> awaiting_trust",
    ];
    const EVAL_RUN_CONTRACT: &[&str] = &[
        "created -> resolving",
        "resolving -> generating",
        "generating -> judging",
        "judging -> scoring",
        "generating -> scoring",
        "scoring -> passed",
        "scoring -> failed",
        "created -> errored",
        "resolving -> errored",
        "generating -> errored",
        "judging -> errored",
        "scoring -> errored",
        "created -> cancelled",
        "resolving -> cancelled",
        "generating -> cancelled",
        "judging -> cancelled",
        "scoring -> cancelled",
    ];

    /// The two state names of a "from -> to" line.
    fn split_transition(line: &str) -> (&str, &str) {
        let (from, to) = line.split_once(" -> ").unwrap();
        (from, to)
    }

    /// Every listed transition is allowed, and every allowed transition is listed: the table is exactly the
    /// contract, read through the names.
    fn check_exact<S: Copy + PartialEq + std::fmt::Debug>(
        contract: &[&str],
        states: &[S],
        parse: fn(&str) -> Option<S>,
        allowed: fn(S, S) -> bool,
    ) {
        let mut listed = Vec::new();
        for line in contract {
            let (from, to) = split_transition(line);
            let pair = (parse(from).unwrap(), parse(to).unwrap());
            assert!(allowed(pair.0, pair.1), "{line} should be allowed");
            listed.push(pair);
        }
        for from in states {
            for to in states {
                if allowed(*from, *to) {
                    assert!(
                        listed.contains(&(*from, *to)),
                        "{from:?} -> {to:?} is allowed but not in the contract"
                    );
                }
            }
        }
    }

    /// Names round-trip, are unique and snake_case; `unspecified` has a name but isn't parsed.
    fn check_names<S: Copy + PartialEq + std::fmt::Debug>(
        states: &[S],
        unspecified: S,
        name: fn(S) -> &'static str,
        parse: fn(&str) -> Option<S>,
    ) {
        let mut seen = Vec::new();
        for state in states {
            let text = name(*state);
            assert_eq!(parse(text), Some(*state), "{text}");
            assert!(
                text.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "{text}"
            );
            assert!(!seen.contains(&text), "{text} twice");
            seen.push(text);
        }
        assert_eq!(name(unspecified), UNSPECIFIED_NAME);
        assert_eq!(parse(UNSPECIFIED_NAME), None);
        assert_eq!(parse("Queued"), None);
        assert_eq!(parse(""), None);
    }

    /// Nothing leaves a terminal state, nothing loops on itself, and `unspecified` is in no transition.
    fn check_shape<S: Copy + PartialEq + std::fmt::Debug>(
        states: &[S],
        unspecified: S,
        allowed: fn(S, S) -> bool,
        is_terminal: fn(S) -> bool,
    ) {
        for from in states {
            assert!(!allowed(*from, *from), "{from:?} loops");
            assert!(
                !allowed(*from, unspecified) && !allowed(unspecified, *from),
                "{from:?} and unspecified"
            );
            for to in states {
                assert!(
                    !(is_terminal(*from) && allowed(*from, *to)),
                    "{from:?} is terminal but goes to {to:?}"
                );
            }
        }
        assert!(!is_terminal(unspecified));
    }

    #[test]
    fn job_machine() {
        check_names(JOB_STATES, JobState::Unspecified, job_state_name, parse_job_state);
        check_exact(JOB_CONTRACT, JOB_STATES, parse_job_state, job_transition_allowed);
        check_shape(
            JOB_STATES,
            JobState::Unspecified,
            job_transition_allowed,
            is_terminal_job_state,
        );
        let terminal: Vec<JobState> = JOB_STATES
            .iter()
            .copied()
            .filter(|state| is_terminal_job_state(*state))
            .collect();
        assert_eq!(terminal, [JobState::Succeeded, JobState::Failed, JobState::Cancelled]);
        // A sample of illegal moves: skipping the handshake, reviving a finished job, pausing a lost one.
        assert!(!job_transition_allowed(JobState::Queued, JobState::Running));
        assert!(!job_transition_allowed(JobState::Succeeded, JobState::Running));
        assert!(!job_transition_allowed(JobState::Cancelled, JobState::Queued));
        assert!(!job_transition_allowed(JobState::Lost, JobState::Paused));
        assert!(!job_transition_allowed(JobState::Stopping, JobState::Running));
        assert!(!job_transition_allowed(JobState::Running, JobState::Queued));
    }

    #[test]
    fn run_machine() {
        check_names(RUN_STATES, RunState::Unspecified, run_state_name, parse_run_state);
        check_exact(RUN_CONTRACT, RUN_STATES, parse_run_state, run_transition_allowed);
        check_shape(
            RUN_STATES,
            RunState::Unspecified,
            run_transition_allowed,
            is_terminal_run_state,
        );
        // Failed and cancelled runs can be resumed, so only completed is terminal.
        assert!(is_terminal_run_state(RunState::Completed));
        assert!(!is_terminal_run_state(RunState::Failed));
        assert!(!is_terminal_run_state(RunState::Cancelled));
        assert!(!run_transition_allowed(RunState::Completed, RunState::Resuming));
        assert!(!run_transition_allowed(RunState::Created, RunState::Running));
        assert!(!run_transition_allowed(RunState::Preempted, RunState::Running));
        assert!(!run_transition_allowed(RunState::Failed, RunState::Running));
        assert!(!run_transition_allowed(RunState::Checkpointing, RunState::Verifying));
        // Preempted is ended but resumable: until the owner decides contracts 14 row 25, a resume is its only exit.
        assert!(!run_transition_allowed(RunState::Preempted, RunState::Cancelled));
        assert!(!run_transition_allowed(RunState::Preempted, RunState::Failed));
    }

    // One assertion per edge contracts 9.5 gained with M1-47, each with what takes it: a job can end in any live
    // state (contracts 9.1), so every live run state needs a way to each end.
    #[test]
    fn every_way_a_job_ends_reaches_the_run_in_any_live_state() {
        // A spawn error, a bad launch or a worker that never registered: the daemon records the run failed.
        assert!(run_transition_allowed(RunState::Created, RunState::Failed));
        // job.cancel of a queued job, or a KILL before the first record: the daemon records it cancelled.
        assert!(run_transition_allowed(RunState::Created, RunState::Cancelled));
        // SIGTERM before the job's entry ran: the worker's edge records preempted.
        assert!(run_transition_allowed(RunState::Created, RunState::Preempted));
        // A periodic checkpoint that hits a full disk (RRS-T2001) pauses the run.
        assert!(run_transition_allowed(RunState::Checkpointing, RunState::Paused));
        // SIGTERM mid-checkpoint: the checkpoint finishes, then the run is preempted.
        assert!(run_transition_allowed(RunState::Checkpointing, RunState::Preempted));
        // STOP mid-checkpoint: that checkpoint is the final one.
        assert!(run_transition_allowed(RunState::Checkpointing, RunState::Completed));
        // A crash, an OOM, a SIGKILL the node didn't send, or RRS-T2001 on the SIGTERM path.
        assert!(run_transition_allowed(RunState::Checkpointing, RunState::Failed));
        // CANCEL, or KILL after the grace period.
        assert!(run_transition_allowed(RunState::Checkpointing, RunState::Cancelled));
        // STOP while paused: the run completes, reason stopped.
        assert!(run_transition_allowed(RunState::Paused, RunState::Completed));
        // SIGTERM while paused.
        assert!(run_transition_allowed(RunState::Paused, RunState::Preempted));
        // A crash, a SIGKILL or exit 4 while paused.
        assert!(run_transition_allowed(RunState::Paused, RunState::Failed));
        // CANCEL or KILL while paused.
        assert!(run_transition_allowed(RunState::Paused, RunState::Cancelled));
        // job.cancel of the resume's job before it restored the checkpoint.
        assert!(run_transition_allowed(RunState::Resuming, RunState::Cancelled));
        // SIGTERM before the resume restored the checkpoint.
        assert!(run_transition_allowed(RunState::Resuming, RunState::Preempted));
    }

    #[test]
    fn model_machine() {
        check_names(
            MODEL_STATES,
            ModelState::Unspecified,
            model_state_name,
            parse_model_state,
        );
        check_exact(
            MODEL_CONTRACT,
            MODEL_STATES,
            parse_model_state,
            model_transition_allowed,
        );
        check_shape(
            MODEL_STATES,
            ModelState::Unspecified,
            model_transition_allowed,
            is_terminal_model_state,
        );
        assert!(MODEL_STATES.iter().all(|state| !is_terminal_model_state(*state)));
        assert!(!model_transition_allowed(ModelState::Unloaded, ModelState::Ready));
        assert!(!model_transition_allowed(ModelState::Downloading, ModelState::Loading));
        assert!(!model_transition_allowed(ModelState::Ready, ModelState::Unloaded));
        assert!(!model_transition_allowed(ModelState::Failed, ModelState::Ready));
    }

    #[test]
    fn lease_machine() {
        check_names(
            LEASE_STATES,
            LeaseState::Unspecified,
            lease_state_name,
            parse_lease_state,
        );
        check_exact(
            LEASE_CONTRACT,
            LEASE_STATES,
            parse_lease_state,
            lease_transition_allowed,
        );
        check_shape(
            LEASE_STATES,
            LeaseState::Unspecified,
            lease_transition_allowed,
            is_terminal_lease_state,
        );
        assert!(is_terminal_lease_state(LeaseState::Released) && is_terminal_lease_state(LeaseState::Denied));
        assert!(!lease_transition_allowed(
            LeaseState::Requested,
            LeaseState::ReclaimRequested
        ));
        assert!(!lease_transition_allowed(LeaseState::Denied, LeaseState::Granted));
        assert!(!lease_transition_allowed(
            LeaseState::ReclaimRequested,
            LeaseState::Granted
        ));
    }

    #[test]
    fn node_machine() {
        check_names(NODE_STATES, NodeState::Unspecified, node_state_name, parse_node_state);
        check_exact(NODE_CONTRACT, NODE_STATES, parse_node_state, node_transition_allowed);
        check_shape(
            NODE_STATES,
            NodeState::Unspecified,
            node_transition_allowed,
            is_terminal_node_state,
        );
        assert!(NODE_STATES.iter().all(|state| !is_terminal_node_state(*state)));
        // Every connection is handshaken, and an incompatible node is never connected without an update.
        assert!(!node_transition_allowed(NodeState::Connecting, NodeState::Connected));
        assert!(!node_transition_allowed(NodeState::Incompatible, NodeState::Connected));
        assert!(!node_transition_allowed(NodeState::Unpaired, NodeState::Connecting));
        assert!(!node_transition_allowed(NodeState::Unreachable, NodeState::Connected));
    }

    #[test]
    fn sandbox_machine() {
        check_names(
            SANDBOX_STATES,
            SandboxState::Unspecified,
            sandbox_state_name,
            parse_sandbox_state,
        );
        check_exact(
            SANDBOX_CONTRACT,
            SANDBOX_STATES,
            parse_sandbox_state,
            sandbox_transition_allowed,
        );
        check_shape(
            SANDBOX_STATES,
            SandboxState::Unspecified,
            sandbox_transition_allowed,
            is_terminal_sandbox_state,
        );
        assert!(is_terminal_sandbox_state(SandboxState::Removed));
        assert!(!sandbox_transition_allowed(
            SandboxState::Requested,
            SandboxState::Running
        ));
        assert!(!sandbox_transition_allowed(
            SandboxState::Removed,
            SandboxState::Running
        ));
        assert!(!sandbox_transition_allowed(
            SandboxState::Stopping,
            SandboxState::Running
        ));
        assert!(!sandbox_transition_allowed(
            SandboxState::Orphaned,
            SandboxState::Running
        ));
    }

    #[test]
    fn plugin_machine() {
        check_names(
            PLUGIN_STATES,
            PluginState::Unspecified,
            plugin_state_name,
            parse_plugin_state,
        );
        check_exact(
            PLUGIN_CONTRACT,
            PLUGIN_STATES,
            parse_plugin_state,
            plugin_transition_allowed,
        );
        check_shape(
            PLUGIN_STATES,
            PluginState::Unspecified,
            plugin_transition_allowed,
            is_terminal_plugin_state,
        );
        assert!(is_terminal_plugin_state(PluginState::Uninstalled));
        // Trust can't be skipped, and an uninstalled plugin is gone.
        assert!(!plugin_transition_allowed(
            PluginState::Resolving,
            PluginState::Installing
        ));
        assert!(!plugin_transition_allowed(
            PluginState::Discovered,
            PluginState::Enabled
        ));
        assert!(!plugin_transition_allowed(
            PluginState::Uninstalled,
            PluginState::Enabled
        ));
        assert!(!plugin_transition_allowed(
            PluginState::Enabled,
            PluginState::Uninstalled
        ));
    }

    #[test]
    fn eval_run_machine() {
        check_names(
            EVAL_RUN_STATES,
            EvalRunState::Unspecified,
            eval_run_state_name,
            parse_eval_run_state,
        );
        check_exact(
            EVAL_RUN_CONTRACT,
            EVAL_RUN_STATES,
            parse_eval_run_state,
            eval_run_transition_allowed,
        );
        check_shape(
            EVAL_RUN_STATES,
            EvalRunState::Unspecified,
            eval_run_transition_allowed,
            is_terminal_eval_run_state,
        );
        assert!(!eval_run_transition_allowed(
            EvalRunState::Created,
            EvalRunState::Passed
        ));
        assert!(!eval_run_transition_allowed(
            EvalRunState::Judging,
            EvalRunState::Passed
        ));
        assert!(!eval_run_transition_allowed(
            EvalRunState::Failed,
            EvalRunState::Scoring
        ));
        assert!(!eval_run_transition_allowed(
            EvalRunState::Passed,
            EvalRunState::Errored
        ));
    }
}
