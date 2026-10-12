"""The plugin registry (rastai_plugin/registry.py): register, find and names per kind; an unknown kind or a repeated
name refused (layout frontend-and-plugins.md 11.1)."""

import pytest
from rastai_formats.generated.plugin import PluginContributions
from rastai_plugin.catalog.generated import RRS_P0103
from rastai_plugin.manifest import ID_FIELD_BY_KIND

from rastai_plugin import REGISTRY_KINDS, Metric, PluginError, PluginRegistry, Recipe

# Contracts 5.5's contribution keys whose implementations are Python base classes (layout 10.1).
EXPECTED_KINDS = {
    "recipes",
    "metrics",
    "lenses",
    "converters",
    "reward_functions",
    "node_types",
    "importers",
    "exporters",
    "sae_loaders",
    "samplers",
    "benchmark_loaders",
    "data_sources",
}


class WordCountMetric(Metric):
    name = "word_count.words"


class TinyRecipe(Recipe):
    name = "tiny"


def test_registry_kinds_are_the_python_contribution_keys() -> None:
    assert set(REGISTRY_KINDS) == EXPECTED_KINDS
    assert len(REGISTRY_KINDS) == len(EXPECTED_KINDS)
    # Each kind is a field of the manifest's contributions and has its id field there.
    contribution_fields = set(PluginContributions.__dataclass_fields__)
    assert set(REGISTRY_KINDS) <= contribution_fields
    assert set(ID_FIELD_BY_KIND) == set(REGISTRY_KINDS)


def test_register_then_find_returns_the_same_object() -> None:
    registry = PluginRegistry()
    metric = WordCountMetric()
    registry.register("metrics", "word_count.words", metric)
    assert registry.find("metrics", "word_count.words") is metric


def test_find_of_a_missing_name_is_none() -> None:
    registry = PluginRegistry()
    registry.register("metrics", "word_count.words", WordCountMetric())
    assert registry.find("metrics", "word_count.letters") is None
    # The same name under another kind is another entry.
    assert registry.find("recipes", "word_count.words") is None


def test_names_are_sorted_and_kept_per_kind() -> None:
    registry = PluginRegistry()
    registry.register("recipes", "sft", TinyRecipe())
    registry.register("recipes", "classification", TinyRecipe())
    registry.register("metrics", "eval.exact_match", WordCountMetric())
    assert registry.names("recipes") == ["classification", "sft"]
    assert registry.names("metrics") == ["eval.exact_match"]
    assert registry.names("lenses") == []


def test_every_kind_starts_empty() -> None:
    registry = PluginRegistry()
    for kind in REGISTRY_KINDS:
        assert registry.names(kind) == []


def test_registries_are_separate() -> None:
    first = PluginRegistry()
    second = PluginRegistry()
    first.register("recipes", "sft", TinyRecipe())
    assert second.names("recipes") == []


def test_unknown_kind_is_refused_with_p0103() -> None:
    registry = PluginRegistry()
    with pytest.raises(PluginError) as caught:
        registry.register("metric", "word_count.words", WordCountMetric())
    assert caught.value.code == RRS_P0103
    assert "'metric'" in caught.value.message


def test_repeated_name_is_refused_with_p0103_and_keeps_the_first() -> None:
    registry = PluginRegistry()
    first = WordCountMetric()
    registry.register("metrics", "word_count.words", first)
    with pytest.raises(PluginError) as caught:
        registry.register("metrics", "word_count.words", WordCountMetric())
    assert caught.value.code == RRS_P0103
    assert "twice" in caught.value.message
    assert registry.find("metrics", "word_count.words") is first


def test_find_and_names_with_an_unknown_kind_are_a_bug_in_the_caller() -> None:
    registry = PluginRegistry()
    with pytest.raises(ValueError):
        registry.find("metric", "word_count.words")
    with pytest.raises(ValueError):
        registry.names("metric")
