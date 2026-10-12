# Op goldens

One golden file per ModelSpec op, `<op>.json` (contracts §5.3 and §12.5). Each number here has one owner: the file. The Rust shape inference in `rastai-formats` (`crates/rastai-formats/tests/op_shapes.rs`), the Python compiler (`python/rastai_models/tests/test_op_fixtures.py`, M1-35) and Rust live stats (`compute_design_stats`, M6-E02) must all match it, and a disagreement is a bug in whichever side differs (contracts §2.4 rule 10).

## Format

```json
{
  "op": "conv1d",
  "flop_convention": "flops counts one forward pass over input_shapes, ...",
  "cases": [
    {
      "name": "textcnn_first_layer",
      "attributes": { "in_channels": 128, "out_channels": 64, "kernel_size": 3, "padding": 1 },
      "input_shapes": [[64, 128, 128]],
      "output_shapes": [[64, 128, 64]],
      "parameter_count": 24640,
      "flops": 402653184
    }
  ]
}
```

- `attributes` are the graph node's `attributes` as a `.rrsmodel` writes them; an attribute left out takes its default (`documents/model_ops.rs` describes each op's).
- `input_shapes` lists the op's inputs in `to_input` order. Sequence tensors are channels-last, `[batch, length, channels]`, the layout an embedding gives; an embedding's input holds token ids.
- `output_shapes` lists the op's outputs; every op so far has one.
- `parameter_count` is the number of trainable values the op holds, biases included.

## The FLOP convention

`flops` counts one forward pass over `input_shapes`, as `torch.utils.flop_counter.FlopCounterMode` counts it: 2 per multiply-accumulate of a matrix multiplication or convolution. Bias adds, activations, pooling, dropout, embedding lookups and concatenation count 0. So a `conv1d` gives `2 × batch × out_length × out_channels × in_channels × kernel_size`, and a `linear` gives `2 × (the product of the leading sizes) × in_features × out_features`. Every file states the convention in its `flop_convention` field, word for word the same.

## Where the numbers come from

They were written from PyTorch 2.14 on CPU in eval mode: each case built as a channels-last module (`nn.Embedding`, `nn.Conv1d` and `max_pool1d`/`avg_pool1d` between two transposes, `nn.Linear`, `nn.Dropout`, the activation modules, `torch.cat`), run once under `FlopCounterMode`, with the parameter count summed from `module.parameters()`. The TextCNN cases use the sizes of the template `templates/designs/textcnn.rrsmodel` (batch 64, length 128, embedding 128, 64 filters, 4 classes).

## Adding an op

The contract that adds an op to the vocabulary adds its file here, with its attribute description and shape rule, in the same PR. `tests/op_shapes.rs` fails when an op with shapes (`OPS_WITH_SHAPES`) has no file, or a file names an op without shapes, and `test_op_fixtures.py` fails the same way for the compiler's op table.
