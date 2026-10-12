"""The module loader: import a plugin's Python module, call its register() and check the names against the manifest
(layout frontend-and-plugins.md 10.3 and 11.2).

It imports only what is already installed. The node installs a plugin's packages from its lock before the module
ever starts (python layout 1.4 rule 2), so a missing package here is an error (RRS-P3001), never something to fetch.
"""

import importlib
from types import ModuleType

from rastai_formats.generated.plugin import PluginManifest

from rastai_plugin.catalog.generated import RRS_P0001, RRS_P0103, RRS_P3001, RRS_P4001
from rastai_plugin.errors import PluginError
from rastai_plugin.manifest import manifest_ids
from rastai_plugin.registry import REGISTRY_KINDS, PluginRegistry

# The module convention (layout frontend-and-plugins.md 11.2): a plugin module defines register(registry).
REGISTER_FUNCTION_NAME = "register"
# python.module is an absolute dotted module name: names joined by dots, nothing relative and no path.
MODULE_NAME_SEPARATOR = "."


def load_plugin_module(manifest: PluginManifest) -> PluginRegistry:
    """Imports the manifest's Python module, calls its register() into a new registry and returns that registry.

    For every kind, the names register() registered must equal the manifest's ids under that key: a difference, a
    missing register(), a register() that raises, or a PluginError from the registry is RRS-P0103. The registry is
    the plugin's own, so names other code registered can't hide a difference."""
    module_name = python_module_name(manifest)
    module = import_plugin_module(manifest.plugin_id, module_name)
    registry = PluginRegistry()
    call_register(manifest.plugin_id, module, registry)
    check_names_match_manifest(manifest, registry)
    return registry


def python_module_name(manifest: PluginManifest) -> str:
    if manifest.python is None or not manifest.python.module:
        reason = "it has no python.module, so there is no Python module to load"
        raise PluginError(RRS_P0001, f"the manifest of {manifest.plugin_id} is invalid: {reason}")
    return manifest.python.module


def import_plugin_module(plugin_id: str, module_name: str) -> ModuleType:
    check_module_name(plugin_id, module_name)
    try:
        return importlib.import_module(module_name)
    except ModuleNotFoundError as error:
        # The module itself, or a package it imports, isn't installed in this environment.
        missing = error.name or module_name
        raise PluginError(RRS_P3001, f"{missing}, needed by the plugin {plugin_id}, isn't installed") from error
    except ImportError as error:
        # Every module was found, but a name the plugin imports from one of them isn't there. The node installs the
        # exact versions the plugin's lock names, so the package is the one the author chose: the name is the
        # plugin's own mistake (a typo, or code written against another version), which is P4001's "report it to
        # the plugin's author", not P3001's "install the dependency".
        reason = f"{module_name} can't be imported: {error}"
        raise PluginError(RRS_P4001, f"the plugin module of {plugin_id} crashed: {reason}") from error
    except Exception as error:
        # Any other failure while the module's top level runs is the module crashing on import.
        reason = f"importing {module_name} raised {type(error).__name__}: {error}"
        raise PluginError(RRS_P4001, f"the plugin module of {plugin_id} crashed: {reason}") from error


def check_module_name(plugin_id: str, module_name: str) -> None:
    """A python.module that isn't an absolute dotted name ('.foo', '../../x', '') is the manifest's mistake (RRS-P0001).
    Handed to importlib, it would fail with a TypeError or a ValueError that reads as the module crashing (P4001)."""
    for part in module_name.split(MODULE_NAME_SEPARATOR):
        if not part.isidentifier():
            reason = f"python.module {module_name!r} isn't an absolute dotted Python module name"
            raise PluginError(RRS_P0001, f"the manifest of {plugin_id} is invalid: {reason}")


def call_register(plugin_id: str, module: ModuleType, registry: PluginRegistry) -> None:
    # The register function is looked up by its conventional name; a plugin module is found only by that name.
    register_function = getattr(module, REGISTER_FUNCTION_NAME, None)
    if not callable(register_function):
        raise PluginError(RRS_P0103, f"the module {module.__name__} of {plugin_id} has no register() function")
    try:
        register_function(registry)
    except PluginError as error:
        raise PluginError(RRS_P0103, f"register() of {plugin_id} failed: {error.message}") from error
    except Exception as error:
        reason = f"{type(error).__name__}: {error}"
        raise PluginError(RRS_P0103, f"register() of {plugin_id} raised {reason}") from error


def check_names_match_manifest(manifest: PluginManifest, registry: PluginRegistry) -> None:
    differences = []
    for kind in REGISTRY_KINDS:
        difference = describe_difference(kind, registry.names(kind), manifest_ids(manifest, kind))
        if difference:
            differences.append(difference)
    if differences:
        raise PluginError(RRS_P0103, f"{manifest.plugin_id}: " + "; ".join(differences))


def describe_difference(kind: str, registered: list[str], declared: list[str]) -> str:
    """One kind's difference in words, or "" when register() and the manifest agree."""
    only_registered = sorted(set(registered) - set(declared))
    only_declared = sorted(set(declared) - set(registered))
    parts = []
    if only_registered:
        parts.append("registered but not in the manifest: " + ", ".join(only_registered))
    if only_declared:
        parts.append("in the manifest but not registered: " + ", ".join(only_declared))
    if not parts:
        return ""
    return f"{kind} " + " and ".join(parts)
