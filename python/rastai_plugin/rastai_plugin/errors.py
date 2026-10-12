"""PluginError: the one error class of rastai_plugin (layout python.md section 1.4 rule 5)."""


class PluginError(Exception):
    """A problem with a plugin, its manifest or the host connection that the caller can't get past on its own.

    `code` is a constant from rastai_plugin.catalog; the edge that catches it (rastai_worker's diagnostics, a plugin
    command handler) turns it into a Diagnostic with that code and `message`. Most codes are RRS-P ones; the host
    connection keeps the worker's RRS-J codes, so a refused Register reads the same as on the node's side."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
