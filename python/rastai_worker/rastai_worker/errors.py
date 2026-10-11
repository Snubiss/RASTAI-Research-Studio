"""WorkerError: the one error class of rastai_worker (layout python.md section 1.4 rule 5)."""


class WorkerError(Exception):
    """A problem the job can't get past on its own. `code` is a constant from rastai_plugin.catalog; the worker's
    edge (diagnostics.py, M1-40) turns the error into a Diagnostic with that code and `message`.

    Most codes are RRS-J ones, but the event log's errors keep the store's RRS-S codes (contracts 7.1), so a corrupt
    log reads the same whether the Rust reader or this writer found it."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
