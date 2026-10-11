"""The generated catalog module (rastai_plugin/catalog/generated.py, written by rastai-catalog-codegen).

It must import, hold plain frozen dataclasses with exactly the fields of contracts 10, and give every code a
constant named after it. tools/codegen/tests/outputs_check.rs checks the generator's escaping on awkward text; this
test checks the checked-in module itself.
"""

import dataclasses

import pytest
from rastai_plugin.catalog import generated

# Contracts 10, "Entry fields", in the catalog JSON's order; and the term fields of layout rust-control-plane.md 2.6.
CATALOG_ENTRY_FIELDS = [
    "code",
    "severity",
    "title",
    "message",
    "basic_text",
    "fix_text",
    "fix_command_id",
    "docs_anchor",
    "retired",
]
TERM_ENTRY_FIELDS = ["term_id", "title", "basic_text", "advanced_text", "docs_anchor"]
SEVERITIES = {"error", "warning", "info"}


def field_names(entry_class: type) -> list[str]:
    return [field.name for field in dataclasses.fields(entry_class)]


def test_entry_classes_have_the_contract_fields() -> None:
    assert field_names(generated.CatalogEntry) == CATALOG_ENTRY_FIELDS
    assert field_names(generated.TermEntry) == TERM_ENTRY_FIELDS


def test_entries_are_frozen() -> None:
    entry = generated.TermEntry(term_id="t", title="T", basic_text="b", advanced_text="a", docs_anchor="d")
    with pytest.raises(dataclasses.FrozenInstanceError):
        entry.title = "changed"


def test_tables_are_tuples_of_entries() -> None:
    assert isinstance(generated.CATALOG, tuple)
    assert isinstance(generated.TERMS, tuple)
    assert all(isinstance(entry, generated.CatalogEntry) for entry in generated.CATALOG)
    assert all(isinstance(term, generated.TermEntry) for term in generated.TERMS)


def test_every_code_has_its_constant_and_a_known_severity() -> None:
    for entry in generated.CATALOG:
        assert getattr(generated, entry.code.replace("-", "_")) == entry.code
        assert entry.severity in SEVERITIES
