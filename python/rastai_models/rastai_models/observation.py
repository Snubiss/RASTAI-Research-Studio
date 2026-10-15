"""PointHook and DecodeReadout: the contract between the runtime and every readout (layout python.md section 2.2).

An observation point is one graph node's output, named by the node's path (layout python.md 2.2). Until block expansion
adds instance paths, a path is the node's id. A node with `observe` false has no point for its main output, but its
extra outputs (layout 2.2) still do. CompiledModel calls the hook at every point, in graph order, and carries on with
the tensor the hook returns. A reader returns its input unchanged; an intervention returns an edited tensor, and the
graph after that point uses the edited one.

The hook runs inside the captured decode step, so it follows capture safety (python/CLAUDE.md rule 9): tensor ops on
preallocated buffers only, no `.item()`, no Python branch on a tensor value, no copy to the host.
"""

from typing import TYPE_CHECKING

import torch

if TYPE_CHECKING:
    # Only for the annotation: the compiled model imports this module, so a runtime import would be circular.
    from rastai_models.compiler.compiled_model import CompiledModel

# How much attention a pass captures (layout 2.2). Capture changes which kernel runs: see attention_kernel.py.
# No attention captured: the fast path.
CAPTURE_NO_ATTENTION = "none"
# The fast output, plus the newest token's attention pattern on the side (the live panel).
CAPTURE_LAST_TOKEN_ATTENTION = "last_token"
# Every query's attention pattern, which only the eager kernel computes.
CAPTURE_FULL_ATTENTION = "full"
CAPTURE_MODES = (CAPTURE_NO_ATTENTION, CAPTURE_LAST_TOKEN_ATTENTION, CAPTURE_FULL_ATTENTION)


class PointHook:
    """Called at every observation point. Returns the tensor the model continues with.

    Readers return the tensor they were given. Interventions (steering, ablation) return an edited tensor. The base
    class is a reader that changes nothing, so a subclass overrides only the points it cares about.
    """

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        return tensor


class DecodeReadout:
    """Plugs a readout into the static decode step. The fused logit lens implements it (layout 2.2), so the
    unembedding matrix is read once per token, not twice."""

    def point_hook(self) -> PointHook:
        raise NotImplementedError

    def finish_step(self, model: "CompiledModel", final_hidden: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """Returns (logits [vocabulary], packet [packet_size] float32), both still on the GPU."""
        raise NotImplementedError


def check_capture_mode(attention_capture: str) -> None:
    """Refuses a capture mode the runtime doesn't know. A wrong name is a bug in the caller, so it raises ValueError."""
    if attention_capture not in CAPTURE_MODES:
        raise ValueError(f"unknown attention capture mode {attention_capture!r}; expected one of {CAPTURE_MODES}")


# The decoder family's standard points (layout 2.2, contracts 5.3). A family names its nodes so that these are the
# nodes' bare paths: the main output of `embedding`, of each block's `attention_out`, `feedforward_out` and
# `residual_out` nodes, and of `final_norm_out`. `i` is the block's index in the model.
EMBEDDING_POINT = "embedding"
FINAL_NORM_POINT = "final_norm_out"
ATTENTION_OUT_NAME = "attention_out"
FEEDFORWARD_OUT_NAME = "feedforward_out"
RESIDUAL_OUT_NAME = "residual_out"
# The attention node's name within a block (`blocks.<i>.attention`), which its sibling points hang off.
ATTENTION_NODE_NAME = "attention"

# The attention node's extra outputs, each a point `<node path>.<output>` (contracts 5.3). The pattern is only emitted
# under full capture and the last-token pattern only under last-token capture; the head outputs always are, when a hook
# is present. A hook sees them in this order, before the node's own point.
ATTENTION_PATTERN_OUTPUT = "attention_pattern"
LAST_TOKEN_ATTENTION_OUTPUT = "last_token_attention"
HEAD_OUTPUTS_OUTPUT = "head_outputs"
ATTENTION_SIBLING_OUTPUTS = (ATTENTION_PATTERN_OUTPUT, LAST_TOKEN_ATTENTION_OUTPUT, HEAD_OUTPUTS_OUTPUT)


def block_point(layer_index: int, name: str) -> str:
    """The point of a block's node `name`: `blocks.<layer_index>.<name>`."""
    return f"blocks.{layer_index}.{name}"


def decoder_standard_points(layer_count: int) -> list[str]:
    """The standard points of a decoder with `layer_count` blocks, in the order a pass reaches them, without the
    attention node's extra outputs: the embedding, then each block's attention, feed-forward and residual points,
    then the final norm."""
    points = [EMBEDDING_POINT]
    for layer_index in range(layer_count):
        points.append(block_point(layer_index, ATTENTION_OUT_NAME))
        points.append(block_point(layer_index, FEEDFORWARD_OUT_NAME))
        points.append(block_point(layer_index, RESIDUAL_OUT_NAME))
    points.append(FINAL_NORM_POINT)
    return points


def attention_sibling_point(layer_index: int, output_name: str) -> str:
    """The point of block `layer_index`'s attention node's extra output `output_name`."""
    return block_point(layer_index, f"{ATTENTION_NODE_NAME}.{output_name}")
