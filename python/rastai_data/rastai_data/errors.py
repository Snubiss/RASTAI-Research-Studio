"""DataError: the one error class of rastai_data (layout python.md section 1.4 rule 5)."""

# The catalog codes this package raises so far (contracts 10.3). They are written out here until the generated
# rastai_plugin.catalog exists (M1-12 generates it, M1-37 fills the package); then these two lines become an import.
RRS_D1001 = "RRS-D1001"  # File format not recognized: a row that can't be read as JSON Lines or CSV
RRS_D3001 = "RRS-D3001"  # Source missing or not readable


class DataError(Exception):
    """A data problem the user can fix. The worker edge turns it into a Diagnostic with `code` and `message`."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
