//! The `job` area's types (contracts 6.4, 9.1): `job.list`, `job.get` and `job.cancel`, and `JobSummary`, the one
//! row of the `jobs` table they all return.
//!
//! Job kinds and states are the proto enums' names (contracts 9.1), written as text. They aren't listed again here:
//! the names belong to `rastai-proto` and `rastai-core`'s states.rs, and the handler parses them there.

use serde::{Deserialize, Serialize};

use crate::commands::{command_type, positional};
use crate::describe::types::{FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

const LIST_COMMAND_ID: &str = "job.list";
const GET_COMMAND_ID: &str = "job.get";
const CANCEL_COMMAND_ID: &str = "job.cancel";
const JOB_SUMMARY_TYPE: &str = "JobSummary";

/// One job, as the `jobs` table holds it (contracts 8.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub job_id: String,
    /// Empty for a job that has no run (a `daemon_task` such as `plugin.install`).
    #[serde(default)]
    pub run_id: String,
    pub node_id: String,
    /// A job kind of contracts 9.1, such as `train`.
    pub kind: String,
    /// A job state of contracts 9.1, such as `running`.
    pub state: String,
    /// The catalog code the job failed with; empty unless it failed.
    #[serde(default)]
    pub error_code: String,
}

/// Which jobs to list; with no filter, every job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobListArgs {
    /// Only this run's jobs.
    #[serde(default)]
    pub run_id: String,
    /// Only jobs in this state.
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobListResult {
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobGetArgs {
    pub job_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobGetResult {
    pub job: JobSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelArgs {
    pub job_id: String,
}

/// The job right after the request: `cancelled` when it hadn't started, else `stopping` until it exits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelResult {
    pub job: JobSummary,
}

pub fn describe_job_summary() -> TypeDescription {
    let fields = vec![
        field("job_id", FieldKind::String, REQUIRED, "The job's id."),
        field(
            "run_id",
            FieldKind::String,
            OPTIONAL,
            "The run it belongs to; empty for a job without a run.",
        ),
        field("node_id", FieldKind::String, REQUIRED, "The node it runs on."),
        field("kind", FieldKind::String, REQUIRED, "Its job kind, such as train."),
        field("state", FieldKind::String, REQUIRED, "Its job state, such as running."),
        field(
            "error_code",
            FieldKind::String,
            OPTIONAL,
            "The catalog code it failed with; empty unless it failed.",
        ),
    ];
    command_type(
        JOB_SUMMARY_TYPE,
        LIST_COMMAND_ID,
        "One job: its run, node, kind and state.",
        fields,
    )
}

pub fn describe_job_list_args() -> TypeDescription {
    let fields = vec![
        field("run_id", FieldKind::String, OPTIONAL, "Only this run's jobs."),
        field(
            "state",
            FieldKind::String,
            OPTIONAL,
            "Only jobs in this job state, such as running.",
        ),
    ];
    command_type(
        "JobListArgs",
        LIST_COMMAND_ID,
        "Which jobs to list; with no filter, every job.",
        fields,
    )
}

pub fn describe_job_list_result() -> TypeDescription {
    let fields = vec![field(
        "jobs",
        list_of(FieldKind::Type(JOB_SUMMARY_TYPE.to_string())),
        REQUIRED,
        "The jobs, newest first.",
    )];
    command_type("JobListResult", LIST_COMMAND_ID, "The jobs that matched.", fields)
}

pub fn describe_job_get_args() -> TypeDescription {
    let fields = vec![job_id_field("The job to show.")];
    command_type("JobGetArgs", GET_COMMAND_ID, "Which job to show.", fields)
}

pub fn describe_job_get_result() -> TypeDescription {
    let fields = vec![job_field("The job.")];
    command_type("JobGetResult", GET_COMMAND_ID, "One job.", fields)
}

pub fn describe_job_cancel_args() -> TypeDescription {
    let fields = vec![job_id_field("The job to cancel.")];
    command_type(
        "JobCancelArgs",
        CANCEL_COMMAND_ID,
        "Which job to cancel. It stops at the next step boundary, without a new checkpoint.",
        fields,
    )
}

pub fn describe_job_cancel_result() -> TypeDescription {
    let fields = vec![job_field(
        "The job right after the request: cancelled if it hadn't started, else stopping until it exits.",
    )];
    command_type(
        "JobCancelResult",
        CANCEL_COMMAND_ID,
        "The job after the cancel request.",
        fields,
    )
}

/// `rastai job get <job_id>`: the id is the first positional argument.
fn job_id_field(doc: &str) -> FieldDescription {
    positional(0, field("job_id", FieldKind::String, REQUIRED, doc))
}

fn job_field(doc: &str) -> FieldDescription {
    field("job", FieldKind::Type(JOB_SUMMARY_TYPE.to_string()), REQUIRED, doc)
}
