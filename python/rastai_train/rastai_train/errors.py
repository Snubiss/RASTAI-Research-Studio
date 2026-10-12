"""TrainError: the one error class of rastai_train (layout python.md section 1.4 rule 5)."""

import json

from rastai_formats import FormatsValidationError


class TrainError(Exception):
    """A training problem the user can fix. `code` is a constant from rastai_plugin.catalog (RRS-T, or the wheel's own
    code for a design it refused); the worker edge turns the error into a Diagnostic with that code and `message`."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


def train_error_from_refusal(error: FormatsValidationError, file_name: str) -> TrainError:
    """The wheel's refusal of a document as a TrainError with the wheel's code, so a design reads the same whether
    Rust or Python opened it. The diagnostic's path is a JSON pointer into the file, so the file's name goes first."""
    first_problem = json.loads(error.args[0])[0]
    place = file_name + first_problem["path"]
    return TrainError(first_problem["code"], f"{place}: {first_problem['message']}")
