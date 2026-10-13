//! Commands over daemon.sock, as the CLI sends them: `CommandService.Execute` in Connect JSON (contracts 3.3, 4.2),
//! with the proto field names, which the daemon reads and writes.
//! The gate admits a same-user peer on daemon.sock as the owner, so no token is needed. Each request runs on a
//! current-thread runtime of its own, so the rows stay plain threads that poll.

use rastai_core::paths::{DataPaths, daemon_socket_path};
use rastai_transport::http_client::{HttpRequest, HttpTarget, send_request};
use serde_json::{Value, json};

use crate::kill_matrix::scenario::Scenario;

const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
const JSON_CONTENT_TYPE: &str = "application/json";
/// Long enough for any command of a row; a request to a daemon that stopped at a pause point ends when it is killed.
const REQUEST_TIMEOUT_MS: u64 = 600_000;
/// The job states nothing moves on from (contracts 9.1).
const FINAL_JOB_STATES: &[&str] = &["succeeded", "failed", "cancelled"];

/// What `Execute` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandAnswer {
    pub ok: bool,
    pub result: Value,
    pub job_id: String,
    pub codes: Vec<String>,
}

/// Runs one command. The error says why no answer came: the daemon isn't serving, or it went away mid-request.
pub fn execute(
    paths: &DataPaths,
    command_id: &str,
    arguments: &Value,
    idempotency_key: &str,
) -> Result<CommandAnswer, String> {
    let body = json!({
        "command_id": command_id,
        "arguments_json": arguments.to_string(),
        "idempotency_key": idempotency_key,
    });
    let request = HttpRequest {
        method: "POST".to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: daemon_socket_path(paths),
        },
        path: EXECUTE_PATH.to_string(),
        headers: vec![("content-type".to_string(), JSON_CONTENT_TYPE.to_string())],
        body: body.to_string().into_bytes(),
        timeout_ms: REQUEST_TIMEOUT_MS,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let response = runtime
        .block_on(send_request(request))
        .map_err(|error| format!("{error:?}"))?;
    if response.status != 200 {
        return Err(format!(
            "HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    let answer: Value = serde_json::from_slice(&response.body).map_err(|error| error.to_string())?;
    Ok(read_answer(&answer))
}

/// The answer's fields, under their proto names as the daemon writes them (http/connect_json.rs); a field at its
/// default may be left out.
fn read_answer(answer: &Value) -> CommandAnswer {
    let result_text = answer["result_json"].as_str().unwrap_or_default();
    let mut codes = Vec::new();
    for diagnostic in answer["diagnostics"].as_array().cloned().unwrap_or_default() {
        codes.push(diagnostic["code"].as_str().unwrap_or_default().to_string());
    }
    CommandAnswer {
        ok: answer["ok"].as_bool().unwrap_or(false),
        result: serde_json::from_str(result_text).unwrap_or(Value::Null),
        job_id: answer["job_id"].as_str().unwrap_or_default().to_string(),
        codes,
    }
}

/// A command that must succeed.
pub fn execute_ok(paths: &DataPaths, command_id: &str, arguments: &Value) -> CommandAnswer {
    let answer = execute(paths, command_id, arguments, "").unwrap();
    assert!(answer.ok, "{command_id} failed: {:?}", answer.codes);
    answer
}

/// Whether the daemon answers `system.status` over daemon.sock: it has started and serves.
pub fn daemon_answers(paths: &DataPaths) -> bool {
    matches!(execute(paths, "system.status", &json!({}), ""), Ok(answer) if answer.ok)
}

/// `train.start` of the scenario's arguments: the run id and the job id.
pub fn start_training(paths: &DataPaths, arguments: &Value) -> (String, String) {
    let answer = execute_ok(paths, "train.start", arguments);
    let run_id = answer.result["run_id"].as_str().unwrap().to_string();
    (run_id, answer.job_id)
}

/// The job as `job.get` shows it: state, error_code, exit_code.
pub fn job_summary(paths: &DataPaths, job_id: &str) -> Value {
    execute_ok(paths, "job.get", &json!({"job_id": job_id})).result["job"].clone()
}

/// The job's state as `job.get` shows it, "" while the daemon doesn't answer.
pub fn job_state(paths: &DataPaths, job_id: &str) -> String {
    match execute(paths, "job.get", &json!({"job_id": job_id}), "") {
        Ok(answer) if answer.ok => answer.result["job"]["state"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

/// Waits until the job is in `state` (`job.get`). A job that ends in another final state fails the row at once.
pub fn wait_for_job_state(scenario: &Scenario, job_id: &str, state: &str) {
    let what = format!("job {job_id} to be {state}");
    scenario.wait_until(&what, || {
        let found = job_state(&scenario.paths, job_id);
        let ended_otherwise = FINAL_JOB_STATES.contains(&found.as_str()) && found != state;
        assert!(
            !ended_otherwise,
            "job {job_id} ended {found}, not {state}\n{}",
            scenario.describe()
        );
        found == state
    });
}
