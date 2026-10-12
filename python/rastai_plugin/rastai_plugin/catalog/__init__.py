"""The error and term catalog for Python (contracts 10).

generated.py is written by `rastai-catalog-codegen` from catalog/ and the first-party plugin catalogs, and never
edited by hand: one constant per code (RRS_E1003 = "RRS-E1003"), the entries as frozen dataclasses (CATALOG) and the
plain-English terms (TERMS). Error classes take their `code` from those constants.

The lookup below is how an edge that turns an error into a Diagnostic finds the entry's severity and fix text.
"""

from rastai_plugin.catalog.generated import CATALOG, CatalogEntry

# Every entry by its code, built once on import: the catalog never changes while a process runs.
CATALOG_BY_CODE = {entry.code: entry for entry in CATALOG}


def find_catalog_entry(code: str) -> CatalogEntry | None:
    """The catalog entry of a code, or None for a code the generated catalog doesn't have, such as a third-party
    plugin's PLG- code."""
    return CATALOG_BY_CODE.get(code)
