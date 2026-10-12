"""The module loader and the manifest reader (rastai_plugin/module_loader.py, manifest.py): a plugin module whose
register() matches its manifest loads; an id mismatch, a missing register() and a register() that raises are
RRS-P0103 (layout frontend-and-plugins.md 10.3 and 11.1).

Each test writes a tiny plugin module and its rastai-plugin.json into a temporary folder and reads the manifest
through the rastai_formats wheel, the same path a real plugin takes. Module names differ per test, because Python
keeps an imported module for the rest of the run.
"""

import json
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_E1003, RRS_P0001, RRS_P0103, RRS_P3001, RRS_P4001

from rastai_plugin import PluginError, PluginRegistry, load_plugin_module, manifest_ids, read_manifest

MANIFEST_FILE_NAME = "rastai-plugin.json"
PLUGIN_ID = "word_count"

# A plugin module that registers one metric and one recipe, the way a real one does (layout 11.2).
MATCHING_MODULE = """
from rastai_plugin import Metric, Recipe


class WordsMetric(Metric):
    name = "word_count.words"


class ShortRecipe(Recipe):
    name = "word_count.short"


def register(registry):
    registry.register("metrics", "word_count.words", WordsMetric())
    registry.register("recipes", "word_count.short", ShortRecipe())
"""
EXTRA_NAME_MODULE = """
def register(registry):
    registry.register("metrics", "word_count.words", object())
    registry.register("metrics", "word_count.letters", object())
"""
NO_REGISTER_MODULE = """
def handle_command(call, host):
    return None
"""
RAISING_REGISTER_MODULE = """
def register(registry):
    raise RuntimeError("the vocabulary file is missing")
"""
REPEATED_NAME_MODULE = """
def register(registry):
    registry.register("metrics", "word_count.words", object())
    registry.register("metrics", "word_count.words", object())
"""
CRASHING_IMPORT_MODULE = """
raise ZeroDivisionError("broken at import")
"""
MISSING_DEPENDENCY_MODULE = """
import rastai_test_package_that_is_not_installed


def register(registry):
    pass
"""
# The package is installed but the name isn't in it: a typo in the plugin, not a missing package.
MISSING_NAME_MODULE = """
from os import name_that_does_not_exist_in_os


def register(registry):
    pass
"""


def manifest_document(module_name: str, contributes: dict) -> dict:
    return {
        "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
        "type": "rastai.plugin",
        "schema_version": 1,
        "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
        "plugin_id": PLUGIN_ID,
        "display_name": "Word count",
        "version": "1.0.0",
        "engines": {"rastai": "^1.0"},
        "requires_capabilities": ["eval.metric@1", "train.recipe@1"],
        "contributes": contributes,
        "python": {"module": module_name},
    }


def matching_contributions() -> dict:
    return {
        "metrics": [{"metric_id": "word_count.words", "title": "Words"}],
        "recipes": [{"recipe_id": "word_count.short", "title": "Short"}],
    }


def write_plugin(folder: Path, module_name: str, module_source: str, document: dict) -> Path:
    """Writes the module as <folder>/modules/<module_name>.py and the manifest beside it; returns the manifest."""
    modules_folder = folder / "modules"
    modules_folder.mkdir(exist_ok=True)
    (modules_folder / f"{module_name}.py").write_text(module_source, encoding="utf-8")
    manifest_path = folder / MANIFEST_FILE_NAME
    manifest_path.write_text(json.dumps(document, indent=2), encoding="utf-8")
    return manifest_path


@pytest.fixture
def plugin_folder(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Path]:
    """A folder whose modules/ is on sys.path for this test only; the modules a test imports are forgotten after."""
    (tmp_path / "modules").mkdir()
    monkeypatch.syspath_prepend(str(tmp_path / "modules"))
    modules_before = set(sys.modules)
    yield tmp_path
    for name in set(sys.modules) - modules_before:
        if name.startswith("loader_fixture_"):
            del sys.modules[name]


def load_written_plugin(folder: Path, module_name: str, module_source: str, contributes: dict) -> PluginRegistry:
    document = manifest_document(module_name, contributes)
    manifest_path = write_plugin(folder, module_name, module_source, document)
    return load_plugin_module(read_manifest(manifest_path))


def test_matching_names_load_into_the_plugins_own_registry(plugin_folder: Path) -> None:
    registry = load_written_plugin(plugin_folder, "loader_fixture_matching", MATCHING_MODULE, matching_contributions())
    assert registry.names("metrics") == ["word_count.words"]
    assert registry.names("recipes") == ["word_count.short"]
    assert registry.find("metrics", "word_count.words").name == "word_count.words"


def test_an_id_mismatch_is_p0103_naming_both_sides(plugin_folder: Path) -> None:
    contributes = {"metrics": [{"metric_id": "word_count.words"}, {"metric_id": "word_count.sentences"}]}
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_mismatch", EXTRA_NAME_MODULE, contributes)
    assert caught.value.code == RRS_P0103
    message = caught.value.message
    assert "metrics registered but not in the manifest: word_count.letters" in message
    assert "in the manifest but not registered: word_count.sentences" in message


def test_a_kind_registered_but_not_declared_is_p0103(plugin_folder: Path) -> None:
    contributes = {"metrics": [{"metric_id": "word_count.words"}]}
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_undeclared_kind", MATCHING_MODULE, contributes)
    assert caught.value.code == RRS_P0103
    assert "recipes registered but not in the manifest: word_count.short" in caught.value.message


def test_a_missing_register_is_p0103(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_no_register", NO_REGISTER_MODULE, matching_contributions())
    assert caught.value.code == RRS_P0103
    assert "no register()" in caught.value.message


def test_a_register_that_raises_is_p0103(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_raising", RAISING_REGISTER_MODULE, matching_contributions())
    assert caught.value.code == RRS_P0103
    assert "RuntimeError: the vocabulary file is missing" in caught.value.message


def test_a_registry_refusal_inside_register_is_p0103(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_repeated", REPEATED_NAME_MODULE, matching_contributions())
    assert caught.value.code == RRS_P0103
    assert "twice" in caught.value.message


def test_a_module_that_isnt_installed_is_p3001_and_nothing_is_installed(plugin_folder: Path) -> None:
    document = manifest_document("loader_fixture_absent", matching_contributions())
    manifest_path = plugin_folder / MANIFEST_FILE_NAME
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(PluginError) as caught:
        load_plugin_module(read_manifest(manifest_path))
    assert caught.value.code == RRS_P3001
    assert "loader_fixture_absent" in caught.value.message
    assert "loader_fixture_absent" not in sys.modules


def test_a_missing_dependency_of_the_module_is_p3001(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(
            plugin_folder, "loader_fixture_dependency", MISSING_DEPENDENCY_MODULE, matching_contributions()
        )
    assert caught.value.code == RRS_P3001
    assert "rastai_test_package_that_is_not_installed" in caught.value.message


def test_a_name_missing_from_an_installed_package_is_p4001_not_p3001(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_missing_name", MISSING_NAME_MODULE, matching_contributions())
    assert caught.value.code == RRS_P4001
    assert "loader_fixture_missing_name can't be imported" in caught.value.message
    assert "name_that_does_not_exist_in_os" in caught.value.message
    assert "isn't installed" not in caught.value.message


def test_a_module_that_crashes_on_import_is_p4001(plugin_folder: Path) -> None:
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_crash", CRASHING_IMPORT_MODULE, matching_contributions())
    assert caught.value.code == RRS_P4001
    assert "ZeroDivisionError" in caught.value.message


def test_a_manifest_without_a_python_module_is_p0001(plugin_folder: Path) -> None:
    document = manifest_document("unused", matching_contributions())
    del document["python"]
    manifest_path = plugin_folder / MANIFEST_FILE_NAME
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(PluginError) as caught:
        load_plugin_module(read_manifest(manifest_path))
    assert caught.value.code == RRS_P0001


def test_read_manifest_raises_the_wheels_own_code(tmp_path: Path) -> None:
    document = manifest_document("unused", matching_contributions())
    document["contributes"]["widgets"] = []
    manifest_path = tmp_path / MANIFEST_FILE_NAME
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(PluginError) as caught:
        read_manifest(manifest_path)
    assert caught.value.code == RRS_E1003
    assert "/contributes/widgets" in caught.value.message


def test_read_manifest_of_a_missing_file_is_p0001(tmp_path: Path) -> None:
    with pytest.raises(PluginError) as caught:
        read_manifest(tmp_path / MANIFEST_FILE_NAME)
    assert caught.value.code == RRS_P0001


def test_manifest_ids_are_sorted_and_an_entry_without_its_id_is_p0001(tmp_path: Path) -> None:
    contributes = {"metrics": [{"metric_id": "word_count.words"}, {"metric_id": "word_count.letters"}]}
    manifest_path = tmp_path / MANIFEST_FILE_NAME
    manifest_path.write_text(json.dumps(manifest_document("unused", contributes)), encoding="utf-8")
    manifest = read_manifest(manifest_path)
    assert manifest_ids(manifest, "metrics") == ["word_count.letters", "word_count.words"]
    assert manifest_ids(manifest, "recipes") == []

    contributes = {"metrics": [{"title": "Words"}]}
    manifest_path.write_text(json.dumps(manifest_document("unused", contributes)), encoding="utf-8")
    with pytest.raises(PluginError) as caught:
        manifest_ids(read_manifest(manifest_path), "metrics")
    assert caught.value.code == RRS_P0001
    assert "metric_id" in caught.value.message


def test_a_repeated_manifest_id_is_p0001_even_when_register_matches(plugin_folder: Path) -> None:
    contributes = matching_contributions()
    contributes["metrics"].append({"metric_id": "word_count.words", "title": "Words again"})
    with pytest.raises(PluginError) as caught:
        load_written_plugin(plugin_folder, "loader_fixture_repeated_id", MATCHING_MODULE, contributes)
    assert caught.value.code == RRS_P0001
    assert "contributes.metrics[1] repeats the metric_id word_count.words" in caught.value.message
