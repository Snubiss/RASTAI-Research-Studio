# ADR-0028: Teach it trains with anchors by default, and Verify always reports drift

**Status:** Accepted, 2026-10-10

## Context

- Teach it is the loop's one-button step: preferred branches or SFT targets → LoRA → Verify (§6, §10). It runs while the chat model stays loaded.
- Spike 03 (40 Q&A pairs, the 5080 capped at 8 GiB, chat model resident):
  - **9.6 s** end to end against a 180 s budget, peaking at **4.7 GiB**;
  - held-out accuracy (new phrasings) went from 20% to 80–90%, and general knowledge stayed at 100%.
  - **Side effects:** the model copied the training data's terse style, and one recipe invented a fact (a "Pedro de Alva" editor for Romeo and Juliet). The keyword checks caught neither.
- Spike 03b: mixing in **anchors** (the base model's own answers to 40 unrelated questions) cut drift on unseen prompts by **83–87%** (KL 2.14 → 0.27–0.36). Held-out accuracy stayed at 80–90%, and style was preserved.
  - Best recipe: facts plus anchors, 8 epochs: 90% held-out in 11.4 s.
  - Generating the anchor answers took 57.4 s once per base model. They are cacheable.
- Memory lessons: full-vocabulary logits at every position cost gigabytes (262k vocabulary). Adapter master weights need float32, or small updates round away in bf16.

## Decision

1. **`teach.start` defaults** (settings `training.*`):
   - anchors on (`anchors_default`), from the default anchor set in `templates/evals/`;
   - loss only at answer positions (`answer_only_loss`);
   - gradient checkpointing on (`gradient_checkpointing`);
   - LoRA rank 16, alpha 32 (`lora`), on every linear projection of every block;
   - float32 adapter master weights with bf16 compute;
   - 8 epochs (the spike's best recipe).
2. **Anchor answers are generated once per base `model_hash`** with greedy decoding, stored as a blob, and reused by every Teach it on that model. Their generation time is not counted in the Teach it time.
3. **Verify runs automatically** after training (training run state `verifying`). It reports three things side by side:
   - the behavior tests, judged by the Evaluation plugin and never by keyword checks alone (ADR-0020);
   - drift: KL from the base model on the fixed general prompt set (`eval.drift_prompt_set`);
   - the model diff, with before and after answers shown side by side.
4. **Budget:** Teach it completes in under 180 s on an 8 GiB cap with the chat model resident. A nightly GPU test checks this on Gemma 3 1B.
5. **Users can turn anchors off** per run (Advanced mode). The run record keeps the choice, and Verify still reports drift.

**Checked by:** the nightly Teach it test (time, peak VRAM, held-out accuracy, drift); the §32 weeks 11–12 exit criteria (the full loop in under 10 minutes from a cold start, and the eval suite catches the invented fact).

## Consequences

**Good**
- New facts stick without changing everything else, and the user sees proof (drift next to the tests).
- The defaults fit an 8 GiB budget beside a resident chat model.
- Anchors need no third-party dataset and carry no data license, because they're the model's own answers.

**Costs**
- Anchors double the training steps (40 → 80 in the spike: 6.0 s → 11.4 s).
- A one-time anchor generation per base model (~1 minute for 1B).
- Drift needs a forward pass of the base model over the prompt set at Verify time.

## Alternatives considered

| Alternative | Why not |
|---|---|
| No anchors | Drift KL 2.14 and a style change in the spike. |
| A KL penalty to the base model during SFT | Needs a reference pass every step. It's an option to measure later, but anchors already cut drift 83–87% cheaply. |
| Mixing in a generic public dataset | Needs a dataset with a license, and doesn't match the model's own voice. |
| Fewer epochs instead of anchors | Recipe C (4 epochs) dropped held-out accuracy to 80%. |
| Keyword tests for Verify | Missed both side effects. |

## Links

- Spec: [§3](../spec/rastai_spec_v5.md#3-positioning), [§6](../spec/rastai_spec_v5.md#6-first-run), [§10](../spec/rastai_spec_v5.md#10-training-and-evaluation) (Teach it, training memory defaults), [§32](../spec/rastai_spec_v5.md#32-first-90-days)
- Contracts: [§6.4 `teach.start`](../architecture/contracts.md#6-commands), [§9.5 training run states](../architecture/contracts.md#9-state-machines), [§11.2 training and eval settings](../architecture/contracts.md#11-configuration)
- Spike: [03 Teach it](../../spike/results/03_teach_it_lora.md), [03b drift](../../spike/results/03b_teach_it_drift.md)
- Related: [ADR-0010](0010-gpu-sharing-vram-leases.md), [ADR-0020](0020-evaluation-first-party-plugin.md), [ADR-0022](0022-ppo-one-base-model-in-vram.md)
