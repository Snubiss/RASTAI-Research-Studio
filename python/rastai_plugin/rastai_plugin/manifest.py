"""A plugin's manifest, read through the rastai_formats wheel, and the ids it declares per registry kind (layout
frontend-and-plugins.md 10.1 and 11.1).

The wheel is the one place a document is validated (python layout 1.4 rule 4), so this file never checks the JSON
itself: it hands the bytes to read_document and turns the first error into a PluginError with the wheel's code.
"""

import json
from pathlib import Path

from rastai_formats.generated.plugin import PluginManifest, plugin_manifest_from_json

from rastai_formats import read_document
from rastai_plugin.catalog.generated import RRS_P0001
from rastai_plugin.errors import PluginError

# The field that holds an entry's id under each `contributes` key with a Python implementation: `<kind>_id`, with
# the kind in the singular (layout frontend-and-plugins.md 10.1). The keys are exactly registry.REGISTRY_KINDS.
ID_FIELD_BY_KIND = {
    "recipes": "recipe_id",
    "metrics": "metric_id",
    "lenses": "lens_id",
    "converters": "converter_id",
    "reward_functions": "reward_function_id",
    "node_types": "node_type_id",
    "importers": "importer_id",
    "exporters": "exporter_id",
    "sae_loaders": "sae_loader_id",
    "samplers": "sampler_id",
    "benchmark_loaders": "benchmark_loader_id",
    "data_sources": "data_source_id",
}
ERROR_SEVERITY = "error"
# The `type` of a plugin manifest. The wheel names a document's type by its `type` field, and a `.json` file name
# doesn't pin one, so a valid document of another type (a rastai.project saved as rastai-plugin.json) passes the
# wheel and is refused here. Same value as rastai-formats' PLUGIN_MANIFEST_TYPE_NAME.
PLUGIN_MANIFEST_TYPE_NAME = "rastai.plugin"


def read_manifest(path: Path) -> PluginManifest:
    """The manifest at `path`, validated by the wheel. A file that can't be read is RRS-P0001; a file the wheel
    refuses raises its first error with the wheel's own code (RRS-E1003 for an unknown field, for example); a valid
    document of another type is RRS-P0001."""
    try:
        data = path.read_bytes()
    except OSError as error:
        raise PluginError(RRS_P0001, f"the manifest {path} can't be read: {error.strerror or error}") from error
    answer = json.loads(read_document(path.name, data))
    raise_first_error(answer["diagnostics"])
    if not answer["document_json"]:
        raise PluginError(RRS_P0001, f"the manifest {path} was refused without a diagnostic")
    document = json.loads(answer["document_json"])
    check_document_type(path, document)
    return plugin_manifest_from_json(document)


def check_document_type(path: Path, document: dict) -> None:
    document_type = document.get("type")
    if document_type != PLUGIN_MANIFEST_TYPE_NAME:
        reason = f"it is a {document_type} document, not a {PLUGIN_MANIFEST_TYPE_NAME} manifest"
        raise PluginError(RRS_P0001, f"the manifest {path} is invalid: {reason}")


def raise_first_error(diagnostics: list[dict]) -> None:
    for diagnostic in diagnostics:
        if diagnostic["severity"] == ERROR_SEVERITY:
            raise PluginError(diagnostic["code"], f"{diagnostic['path']}: {diagnostic['message']}")


def manifest_ids(manifest: PluginManifest, kind: str) -> list[str]:
    """The ids the manifest declares under one registry kind, sorted. An entry without its id, or an id that two
    entries share, is RRS-P0001: the manifest schema leaves these entries open, so the wheel can't catch either.
    register() can't register a name twice, so a repeated id is always a manifest mistake."""
    if kind not in ID_FIELD_BY_KIND:
        raise ValueError(f"unknown registry kind {kind!r}")
    id_field = ID_FIELD_BY_KIND[kind]
    ids = []
    for index, entry in enumerate(contributions_of(manifest, kind)):
        entry_id = entry.get(id_field)
        if not isinstance(entry_id, str) or not entry_id:
            reason = f"contributes.{kind}[{index}] has no {id_field}"
            raise PluginError(RRS_P0001, f"the manifest of {manifest.plugin_id} is invalid: {reason}")
        if entry_id in ids:
            reason = f"contributes.{kind}[{index}] repeats the {id_field} {entry_id}"
            raise PluginError(RRS_P0001, f"the manifest of {manifest.plugin_id} is invalid: {reason}")
        ids.append(entry_id)
    return sorted(ids)


def contributions_of(manifest: PluginManifest, kind: str) -> list[dict]:
    if manifest.contributes is None:
        return []
    # The kinds are the field names of PluginContributions (contracts 5.5), so the field is looked up by name.
    entries = getattr(manifest.contributes, kind)
    if entries is None:
        return []
    return entries
