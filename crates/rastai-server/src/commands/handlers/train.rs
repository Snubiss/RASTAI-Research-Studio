//! `train.start` (contracts 6.4 and 6.5; flows/training-evaluation.md 4.1; flows/platform.md 5.1): a job command.
//! It checks the arguments, plans the run, records the run and its job `queued`, and answers the job id at once;
//! the scheduler launches the job when its node is connected (scheduler/).
//!
//! M1 trains one recipe, `classification`, on a design and a dataset on the embedded node, on the CPU, with fixed
//! batch sizes. Hardware-aware plans, model references, recipe settings and the run's length from its data and epochs
//! join with M5 (training flow 3). Until then a run trains `MAX_STEPS` optimizer steps.
//!
//! The job's `arguments_json` is a `TrainJobArgs` and its `checkpoint_policy_json` the resolved `training.checkpoint`
//! section (contracts 4.4). A dry run returns the plan and starts nothing.

use rastai_core::commands::output::CommandOutput;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::random::random_bytes;
use rastai_core::settings::{ResolvedSettings, setting_text};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::commands::result_json;
use rastai_formats::commands::train::{TrainJobArgs, TrainStartArgs, TrainStartResult, TrainingPlan};
use rastai_formats::documents::run_record::RunKind;
use rastai_proto::node::v1::{JobKind, JobSpec};
use serde_json::Value;

use crate::commands::context::{CommandContext, core_failure};
use crate::scheduler::launch::RunStart;
use crate::scheduler::placement::pick_node;
use crate::scheduler::queue::submit_job;

/// The one recipe M1 trains (TextCNN classification, plan M1-46).
const CLASSIFICATION_RECIPE: &str = "classification";
/// The environment the embedded node runs a CPU train job in (daemon.rs names it).
const CPU_ENVIRONMENT_ID: &str = "cpu";
/// Optimizer steps in an M1 run: about three passes over a 2,000-row dataset at the plan's batch. Recipe settings
/// choose it from the data and the epochs from M5.
const MAX_STEPS: u32 = 400;
/// The M1 plan: full training in fp32 on the CPU (the train job refuses anything else yet), at the batch sizes and
/// length the TextCNN classification recipe was measured with (python layout 5.4).
const TRAINING_METHOD: &str = "full";
const TRAINING_DTYPE: &str = "fp32";
const MICRO_BATCH: u32 = 16;
const GRADIENT_ACCUMULATION: u32 = 1;
const SEQUENCE_LENGTH: u32 = 128;
/// A TextCNN classification run on the CPU stays well under this; the estimates join with ModelSpec analysis (M5).
const MEMORY_ESTIMATE_BYTES: u64 = 256 * 1024 * 1024;
/// About 40 steps a second on one CPU core, plus the start of Python and torch.
const TIME_ESTIMATE_SECONDS: u64 = 15;
const CHECKPOINT_SECTION: &str = "training";
const CHECKPOINT_KEY: &str = "checkpoint";
const DATA_POLICY_SETTING: &str = "privacy.data_policy";
/// How many random bytes the run's seed is drawn from: a seed below 2^32 is exact in every language that reads it.
const SEED_BYTES: usize = 4;
const CODE_RECIPE_NOT_INSTALLED: &str = "RRS-T0002";
const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const EXPECTED_ABSOLUTE_PATH: &str = "an absolute path on the node";

/// Starts a training run (see the top). The answer's `job_id` is the job; the result names the run and its plan.
pub(crate) async fn start(
    context: &CommandContext<'_>,
    arguments: TrainStartArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    check_start_arguments(&arguments)?;
    let plan = cpu_training_plan(&arguments.recipe);
    if context.dry_run {
        return Ok(answer(String::new(), plan, String::new()));
    }
    let node_id = pick_node(context.server)?;
    let run_id = new_id(IdPrefix::Run).map_err(|error| core_failure(&error))?;
    let seed = new_seed()?;
    let spec = train_job_spec(context, &run_id, &arguments, &plan, seed);
    let run = RunStart {
        project_id: context.project_id.clone(),
        kind: RunKind::Train,
        recipe: arguments.recipe.clone(),
        seed: Some(seed),
    };
    let job_id = submit_job(context.server, spec, &node_id, Some(run)).await?;
    Ok(answer(run_id, plan, job_id))
}

/// The recipe must be one this build trains (`RRS-T0002`), and the paths absolute: the job runs on the node, whose
/// working folder isn't the caller's (`RRS-C0002`).
fn check_start_arguments(arguments: &TrainStartArgs) -> Result<(), Vec<Diagnostic>> {
    if arguments.recipe != CLASSIFICATION_RECIPE {
        return Err(vec![diagnostic(
            CODE_RECIPE_NOT_INSTALLED,
            &[("name", &arguments.recipe)],
        )]);
    }
    let mut problems = Vec::new();
    for path in [&arguments.design_path, &arguments.dataset_path] {
        if !std::path::Path::new(path).is_absolute() {
            problems.push(diagnostic(
                CODE_INVALID_ARGUMENTS,
                &[("found", path), ("expected", EXPECTED_ABSOLUTE_PATH)],
            ));
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// M1's one plan (see the top). Its recipe is the one asked for, which the train job checks against its arguments.
fn cpu_training_plan(recipe: &str) -> TrainingPlan {
    TrainingPlan {
        recipe: recipe.to_string(),
        method: TRAINING_METHOD.to_string(),
        dtype: TRAINING_DTYPE.to_string(),
        gpu_indices: Vec::new(),
        micro_batch: MICRO_BATCH,
        gradient_accumulation: GRADIENT_ACCUMULATION,
        sequence_length: SEQUENCE_LENGTH,
        memory_estimate_bytes: MEMORY_ESTIMATE_BYTES,
        time_estimate_seconds: TIME_ESTIMATE_SECONDS,
    }
}

/// The train job's `JobSpec` (contracts 4.4); `submit_job` fills in its id.
fn train_job_spec(
    context: &CommandContext<'_>,
    run_id: &str,
    arguments: &TrainStartArgs,
    plan: &TrainingPlan,
    seed: u64,
) -> JobSpec {
    let job_arguments = TrainJobArgs {
        start_arguments: arguments.clone(),
        plan: plan.clone(),
        max_steps: MAX_STEPS,
        seed,
    };
    let settings = match context.server.settings.read() {
        Ok(settings) => settings.clone(),
        // A panic while the settings were replaced leaves the old or the new value, either whole.
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    JobSpec {
        job_id: String::new(),
        run_id: run_id.to_string(),
        kind: JobKind::Train as i32,
        environment_id: CPU_ENVIRONMENT_ID.to_string(),
        arguments_json: result_json(&job_arguments),
        data_policy: setting_text(&settings, DATA_POLICY_SETTING),
        checkpoint_policy_json: checkpoint_policy_json(&settings),
        ..JobSpec::default()
    }
}

/// The resolved `training.checkpoint` section as JSON text; empty text, every default, when it isn't an object.
fn checkpoint_policy_json(settings: &ResolvedSettings) -> String {
    match settings
        .values
        .get(CHECKPOINT_SECTION)
        .and_then(|section| section.get(CHECKPOINT_KEY))
    {
        Some(Value::Object(section)) => Value::Object(section.clone()).to_string(),
        _ => String::new(),
    }
}

fn new_seed() -> Result<u64, Vec<Diagnostic>> {
    let bytes = random_bytes(SEED_BYTES).map_err(|error| core_failure(&error))?;
    let mut seed: u64 = 0;
    for byte in bytes {
        seed = (seed << 8) | u64::from(byte);
    }
    Ok(seed)
}

fn answer(run_id: String, plan: TrainingPlan, job_id: String) -> CommandOutput {
    let result = TrainStartResult { run_id, plan };
    CommandOutput {
        result_json: result_json(&result),
        job_id,
        ..CommandOutput::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::settings::{default_settings, resolve_settings};

    fn arguments(recipe: &str, design_path: &str) -> TrainStartArgs {
        TrainStartArgs {
            recipe: recipe.to_string(),
            design_path: design_path.to_string(),
            dataset_path: "/data/textcnn_tiny.jsonl".to_string(),
            eval_suite_ids: Vec::new(),
        }
    }

    #[test]
    fn only_the_classification_recipe_on_absolute_paths_starts() {
        assert_eq!(
            check_start_arguments(&arguments("classification", "/d/textcnn.rrsmodel")),
            Ok(())
        );
        let unknown = check_start_arguments(&arguments("sft", "/d/textcnn.rrsmodel")).unwrap_err();
        assert_eq!(unknown[0].code, "RRS-T0002");
        assert!(unknown[0].message.contains("sft"), "{}", unknown[0].message);
        let relative = check_start_arguments(&arguments("classification", "textcnn.rrsmodel")).unwrap_err();
        assert_eq!(relative.len(), 1);
        assert_eq!(relative[0].code, "RRS-C0002");
    }

    // The train job refuses a plan other than full fp32 on the CPU, and a recipe other than its arguments' (M1-46).
    #[test]
    fn the_plan_is_one_the_train_job_runs() {
        let plan = cpu_training_plan("classification");
        assert_eq!((plan.method.as_str(), plan.dtype.as_str()), ("full", "fp32"));
        assert!(plan.gpu_indices.is_empty());
        assert_eq!(plan.recipe, "classification");
    }

    #[test]
    fn the_checkpoint_policy_is_the_resolved_section() {
        let settings = resolve_settings(&[default_settings()]);
        let policy: Value = serde_json::from_str(&checkpoint_policy_json(&settings)).unwrap_or_default();
        assert_eq!(policy["keep_last"], 3);
        assert!(policy.get("preemption_budget_seconds").is_some(), "{policy}");
    }
}
