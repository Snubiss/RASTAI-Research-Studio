"""DataError: the one error class of rastai_data (layout python.md section 1.4 rule 5)."""


class DataError(Exception):
    """A data problem the user can fix. `code` is a constant from rastai_plugin.catalog (RRS-D); the worker edge
    turns the error into a Diagnostic with that code and `message`."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
