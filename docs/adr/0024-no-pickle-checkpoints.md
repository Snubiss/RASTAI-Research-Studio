# ADR-0024: No pickle: weights are safetensors, checkpoints are safetensors plus JSON

**Status:** Accepted, 2026-10-10

## Context

- Pickle runs arbitrary code on load. Shared checkpoints, imported models and plugin files are untrusted input.
- `torch.load(weights_only=True)` narrows the risk but still parses the pickle format, and its allow-list has had bypasses.
- Every recipe needs a resume contract (weights, optimizer state, RNG state, dataloader position) that survives preemption (§10). PyTorch's usual way to save that is `torch.save`, which is pickle.
- The existing TextCNN code (`train_model.py`) saves pickled `.pt` files and must be ported off them (§29).

## Decision

1. **Weights** are `.safetensors`, read and written by our own code (ADR-0029). `.gguf` is written in-house for export only.
2. **A checkpoint is a folder** (contracts §8.4):
   - `weights.safetensors`, or `adapter.safetensors` for LoRA and frozen-base runs;
   - `optimizer.safetensors`: the optimizer's per-parameter tensors, keyed `<parameter name>.<state name>` (for example `blocks.3.attention.query_projection.compress.exp_avg`);
   - `rng.safetensors`: the torch CPU RNG state and each device's CUDA RNG state, as byte tensors;
   - `resume.json` (`rastai.resume/1`): step, epoch, samples seen, dataloader position (dataset hash, split, shuffle seed, epoch, position), scheduler values, non-tensor optimizer values (step counts, hyperparameters) and `recipe_state`.
3. **Atomic save:** write into `step-<n>.partial/`, fsync, rename to `step-<n>/`, then emit `checkpoint.saved`. A crash leaves either the previous checkpoint or the new one, never half of one.
4. **Recipes** implement `resume_state() -> dict` (plain JSON values only) and restore from `resume.json` plus the tensor files. Iterator state is recorded as numbers, never serialized as objects.
5. **Forbidden in our code:**
   - `import pickle`, `cPickle`, `dill`, `joblib`, `shelve`;
   - `torch.load` and `torch.save` in any form;
   - `numpy.load(..., allow_pickle=True)`.
   CI greps for all of them.
6. **Imports:** pickled `.pt` / `.bin` weights in a model repo are refused with an `RRS-E7xxx` code that suggests the repo's safetensors revision. The compat runtime loads with `use_safetensors=True`.
7. **Scope of the rule:** pickle never touches a file, a socket, or data from outside our process tree. Python's `multiprocessing` (used by PyTorch DataLoader workers) pickles our own in-memory objects between a job and its own children. That is not covered by the grep, and it's allowed until the owner rules otherwise (see open questions).

**Checked by:** the CI grep; a resume test per recipe (train N steps, kill, resume, and compare the loss curve with an uninterrupted run within tolerance); a crash test that kills a job mid-checkpoint and resumes from the last complete one; the CPU loop test with TextCNN ported off `.pt` (§29).

## Consequences

**Good**
- Opening any Rastai file, checkpoint or imported model can't run code.
- Checkpoints are inspectable with any safetensors reader, and diffable as JSON.
- Resume is explicit, so preemption-safe resume is testable per recipe.

**Costs**
- Optimizer and scheduler state need hand-written flatten and restore code per optimizer type (AdamW first). PyTorch's `state_dict()` can't simply be saved.
- Some models on the Hub ship only pickled weights. They can't be imported until someone converts them in a trusted environment.
- The grep rule needs care to avoid false positives in comments and docs.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `torch.save` plus `torch.load(weights_only=True)` | Still the pickle format, with a history of bypasses. Breaks the "no pickle anywhere" rule. |
| One safetensors file for everything | Non-tensor state (positions, seeds, scheduler) is awkward as tensors and unreadable to people. |
| An external checkpoint library (orbax, msgpack-based formats) | Off the allowlist, and adds a format nobody else reads. |

## Links

- Spec: [§10](../spec/rastai_spec_v5.md#10-training-and-evaluation) (preemption-safe by default), [§15](../spec/rastai_spec_v5.md#15-file-formats), [§23](../spec/rastai_spec_v5.md#23-validation), [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup)
- Contracts: [§5.2 resume state](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§8.4 checkpoints](../architecture/contracts.md#8-storage), [§12.3 Python rules](../architecture/contracts.md#12-naming-and-code-conventions)
- Related: [ADR-0002](0002-durability-worker-event-logs.md), [ADR-0029](0029-strict-safetensors-contract.md), [ADR-0032](0032-document-files-and-validation.md)
