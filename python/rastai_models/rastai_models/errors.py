"""ModelError: the one error class of rastai_models (layout python.md section 1.4 rule 5)."""

import json

from rastai_formats import FormatsValidationError


class ModelError(Exception):
    """A model problem the user can fix. `code` is a catalog code; the worker edge turns the error into a Diagnostic
    with that code and `message`.

    Most codes are RRS-M ones. A weight file the rastai_formats wheel refused keeps the wheel's own code (an RRS-E60xx
    for a safetensors header), so a file reads the same whether Rust or Python opened it."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


def model_error_from_refusal(error: FormatsValidationError, file_name: str) -> ModelError:
    """The wheel's refusal as a ModelError with the same code and message. Its path is a JSON pointer into the
    header (`#/a/shape`) or empty, so the file's name goes in front of it."""
    first_problem = json.loads(error.args[0])[0]
    place = file_name + first_problem["path"]
    return ModelError(first_problem["code"], f"{place}: {first_problem['message']}")
