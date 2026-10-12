"""rastai_models: the Inspectable runtime (layout python.md section 2). M1-34 landed the safetensors I/O; M1-35 adds
the ModelSpec compiler and the TextCNN blocks."""

from rastai_models.compiler.compile import compile_model
from rastai_models.compiler.compiled_model import CompiledModel
from rastai_models.compiler.weights import BindReport, bind_weights, init_weights
from rastai_models.errors import ModelError
from rastai_models.io.safetensors_reader import SafetensorsFile
from rastai_models.io.safetensors_writer import write_safetensors

__all__ = [
    "BindReport",
    "CompiledModel",
    "ModelError",
    "SafetensorsFile",
    "bind_weights",
    "compile_model",
    "init_weights",
    "write_safetensors",
]
