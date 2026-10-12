# Model fixtures

One folder per model family or design template (contracts §12.5 and §15.8). Each holds `expected_counts.json`: the parameter counts of the compiled model, which the Python compiler (`python/rastai_models/tests/test_compile.py`) and Rust live stats (`crates/rastai-server/tests/modelspec_stats.rs`, M6-E02) both check, so neither side's count is the other's reference (contracts §2.4 rule 10). Tiny random-weight configs for the HF families (M2) join here; their weights are generated at test time and never committed.

## `expected_counts.json`

```json
{
  "design": "templates/designs/textcnn.rrsmodel",
  "parameter_count": 3290692,
  "trainable_parameter_count": 3290692,
  "node_parameter_counts": { "embedding": 3200000, "conv_1": 24640 }
}
```

- `design` is the ModelSpec it counts, relative to the repository root.
- `parameter_count` is every value the model holds, biases included; `trainable_parameter_count` leaves out the nodes whose `train_mode` is `freeze` or `lora` (their base weights are frozen).
- `node_parameter_counts` gives each graph node's count, every node listed; they add up to `parameter_count`.

## Where the numbers come from

| Folder | Counts |
|---|---|
| `textcnn/` | the TextCNN template `templates/designs/textcnn.rrsmodel`: the sum of each node's count in its `fixtures/ops/<op>.json` case (embedding 25,000 × 128; five `conv1d` of 64 filters with kernels 3, 4, 5, 4, 3 and biases; a classifier head of 64 × 4 plus 4 biases), 3,290,692 in all, the figure contracts §15.8 gives the template |

The file is written once from those sizes and then checked against the real modules by `test_compile.py`, never regenerated from the compiler, so a compiler that miscounts fails the test instead of rewriting its own answer.
