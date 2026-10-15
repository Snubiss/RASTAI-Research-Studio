"""rastai_models: the Inspectable runtime (layout python.md section 2). M1-34 landed the safetensors I/O; M1-35 adds
the ModelSpec compiler and the TextCNN blocks; M2-02 adds the observation contract, the forward context, the attention
kernels and the model traits."""

from rastai_models.compiler.compile import compile_model
from rastai_models.compiler.compiled_model import CompiledModel
from rastai_models.compiler.weights import BindReport, bind_weights, init_weights
from rastai_models.errors import ModelError
from rastai_models.forward_context import ForwardContext
from rastai_models.io.safetensors_reader import SafetensorsFile
from rastai_models.io.safetensors_writer import sync_folder, write_safetensors
from rastai_models.model_traits import ModelTraits, model_traits_of
from rastai_models.observation import DecodeReadout, PointHook

__all__ = [
    "BindReport",
    "CompiledModel",
    "DecodeReadout",
    "ForwardContext",
    "ModelError",
    "ModelTraits",
    "PointHook",
    "SafetensorsFile",
    "bind_weights",
    "compile_model",
    "init_weights",
    "model_traits_of",
    "sync_folder",
    "write_safetensors",
]
