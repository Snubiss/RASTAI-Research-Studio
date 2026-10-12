"""Each graph node's `train_mode` (contracts 5.2): `train` (the default), `freeze` or `lora`.

`train` and `freeze` turn the node's parameters' gradients on or off. `lora` trains low-rank adapters beside frozen
base weights, so here it freezes the base weights; the adapters themselves are attached by lora.py (M5), which reads
the node's `lora{rank, alpha}` field when that joins `rastai.model/1` (M5-E03).
"""

from rastai_models.compiler.compiled_model import CompiledModel

TRAIN_MODE_TRAIN = "train"
TRAIN_MODE_FREEZE = "freeze"
TRAIN_MODE_LORA = "lora"
# Whether a node's own parameters get gradients, by train mode.
TRAINS_BASE_PARAMETERS = {
    TRAIN_MODE_TRAIN: True,
    TRAIN_MODE_FREEZE: False,
    TRAIN_MODE_LORA: False,
}


def apply_train_modes(model: CompiledModel) -> None:
    """Sets `requires_grad` on every parameter from its graph node's train mode."""
    for step, op_module in zip(model.graph_steps, model.op_modules, strict=True):
        train_mode = step.node.train_mode or TRAIN_MODE_TRAIN
        trains = TRAINS_BASE_PARAMETERS[train_mode]
        for parameter in op_module.parameters():
            parameter.requires_grad_(trains)
