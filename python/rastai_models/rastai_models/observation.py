"""PointHook and DecodeReadout: the contract between the runtime and every readout (layout python.md section 2.2).

An observation point is one graph node's output, named by the node's path (layout python.md 2.2). Until block expansion
adds instance paths, a path is the node's id. A node with `observe` false is not a point. CompiledModel calls the hook
at every point, in graph order, and carries on with the tensor the hook returns. A reader returns its input unchanged;
an intervention returns an edited tensor, and the graph after that point uses the edited one.

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
