# ADR-0020: Evaluation is a first-party plugin that tests models like code

**Status:** Accepted, 2026-10-10

## Context

- The loop ends in Verify: proof that a fix worked and nothing else broke (§3).
- Spike 03: after Teach it, the model copied the training data's terse style, and one recipe invented a fact (a "Pedro de Alva" editor for Romeo and Juliet). **The keyword checks caught neither.** Drift KL from the base model on a fixed prompt set did separate them: 2.14 without anchors vs 0.27–0.36 with anchors (03b).
- The plugin API must be proven by real first-party use before it's frozen (§2, ADR-0015). Psychology is the first such plugin; Evaluation is the second.
- Agent Gym rewards and eval scores must share one definition, or a harness's reward and its eval drift apart (§12).
- LLM judges are biased (position bias, self-preference), so a passing suite can give false confidence (§28 risk 15).

## Decision

1. **Evaluation is the plugin `eval`**, built only on the public API:
   - folder `plugins/evaluation/`;
   - Python module `rastai_eval`, importing only `rastai` and `rastai_plugin`;
   - UI `@rastai/eval-ui`;
   - commands `eval.*` with scope `eval`;
   - error codes `RRS-V`.
   It ships preinstalled.
2. **It is the reference implementation of the eval-metric extension point.** Every metric is a `Metric` subclass registered by name in the `rastai_plugin` registry: exact match, keyword, regex, LLM judge, Pass@k, Elo (Bradley-Terry), perplexity with bits per byte, drift, and reward-model score. Engines look metrics up by name and never import `rastai_eval` (for example, `rastai_rl` gets `eval.pass_at_k` from the registry).
3. **Model tests:** an `.rrseval` suite holds tests (input, expectation, metric, pass threshold), pinned conversations, judges, benchmarks and a drift section. A suite passes or fails as a whole.
4. **CLI contract:** `rastai eval test <suite> --model <model or checkpoint>`:
   - exit `0` = `passed`;
   - exit `1` = `failed` (tests failed);
   - exit `2` = `errored` (the suite couldn't be evaluated).
   This lets a suite gate a checkpoint, a Teach it result or a CI job.
5. **Suites run on every new checkpoint** (`eval.run_on_new_checkpoint`, default true). Results are stored per checkpoint as `eval.test_result` events, and `eval.result.diff` shows regressions between two runs.
6. **Keyword checks are never the only check on generated text.**
   - A suite whose generated-text tests use only exact, keyword or regex metrics gets an `RRS-V0xxx` warning from validation.
   - Verify always adds drift.
7. **Judges:**
   - pairwise comparisons are run in both orders, and a disagreement counts as a tie;
   - `eval.judge.calibrate` measures agreement with hand labels, and the agreement rate is shown before a judge is trusted;
   - the judge's prompt and verdict are logged with every result (as a blob).
8. **Pass@k** runs n samples per task in the sandbox manager (ADR-0031) and uses the unbiased estimator. **Elo** is fit with Bradley-Terry and shown with bootstrap confidence intervals.
9. **Benchmarks:** an in-house runner. Task definitions (for example lm-eval-harness) are imported as data.

```python
# plugins/evaluation/python/rastai_eval/metrics/drift.py
# The prompt set comes from the setting eval.drift_prompt_set: one fixed set, so drift compares across checkpoints.

class DriftMetric(Metric):
    """Mean KL from the base model's next-token distribution. Caught what keyword checks missed in the spike."""
    name = "eval.drift"

    def score(self, case: EvalCase, output: ModelOutput) -> MetricResult: ...
```

*Note since acceptance (2026-10-10): registry names are namespaced by plugin id (`eval.drift`, `eval.pass_at_k`; plan §10 row 17). Only the examples changed; the decision is unchanged.*

**Checked by:** the §32 weeks 11–12 exit criterion (the suite catches the spike's invented fact that keyword checks miss); `check-imports` showing `rastai_eval` imports only the public API; a CLI test for each exit code; a judge test where swapping the answer order flips the verdict, which must score as a tie.

## Consequences

**Good**
- Verify catches the failures the spike found. Users get a pass/fail they can put in CI.
- The metric extension point is proven by nine real metrics before third parties use it.
- Agent rewards and eval scores share one implementation.

**Costs**
- Judges cost tokens (API) or GPU time (local), and both-order judging doubles that.
- Building only on the public API means public API work comes first.
- Judge calibration needs hand labels from the user before a judge counts as trusted.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Evaluation as core code | The plugin API would go unproven, and third-party metrics would be second-class. |
| lm-eval-harness as a dependency | A large Python dependency off the allowlist. Its task definitions are imported as data instead. |
| Keyword checks for Verify | Missed both failures in the spike. |
| Single-order judging | Position bias goes undetected. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 18, [§3](../spec/rastai_spec_v5.md#3-positioning), [§10](../spec/rastai_spec_v5.md#10-training-and-evaluation), [§12](../spec/rastai_spec_v5.md#12-agent-gym), [§28](../spec/rastai_spec_v5.md#28-risk-register) risk 15
- Contracts: [§1 first-party plugins](../architecture/contracts.md#1-repository-tree), [§6.4 Evaluation commands](../architecture/contracts.md#6-commands), [§9.6 eval run states](../architecture/contracts.md#9-state-machines), [§12.3 Metric](../architecture/contracts.md#12-naming-and-code-conventions)
- Spike: [03 Teach it](../../spike/results/03_teach_it_lora.md), [03b drift](../../spike/results/03b_teach_it_drift.md)
- Related: [ADR-0015](0015-plugins-and-licensing.md), [ADR-0028](0028-teach-it-anchors-and-drift.md), [ADR-0031](0031-sandbox-manager.md)
