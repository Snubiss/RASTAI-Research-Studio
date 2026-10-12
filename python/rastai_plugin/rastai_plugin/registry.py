"""The plugin registry: every extension-point implementation a process knows, by kind and name (contracts 12.3;
layout frontend-and-plugins.md 11.2).

A plain table of tables. Engines and plugin modules fill it with explicit `register` calls (never a decorator), and
engines look implementations up with `find`. Each process builds its own; nothing is shared between processes.
"""

from rastai_plugin.catalog.generated import RRS_P0103
from rastai_plugin.errors import PluginError

# The manifest's `contributes` keys that have Python implementations (contracts 5.5). Every kind is one of these.
REGISTRY_KINDS = (
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
)


class PluginRegistry:
    """Every extension-point implementation this process knows, by kind and name."""

    def __init__(self) -> None:
        # kind -> name -> implementation. Every kind has its table from the start, so an unknown kind is simply a
        # kind with no table.
        self.implementations: dict[str, dict[str, object]] = {}
        for kind in REGISTRY_KINDS:
            self.implementations[kind] = {}

    def register(self, kind: str, name: str, implementation: object) -> None:
        """Adds one implementation. A plugin's register() calls this, so its mistakes are RRS-P0103, the code the
        module loader gives any register() that doesn't match its manifest."""
        if kind not in self.implementations:
            known_kinds = ", ".join(REGISTRY_KINDS)
            raise PluginError(RRS_P0103, f"register() used the unknown kind {kind!r}; the kinds are {known_kinds}")
        table = self.implementations[kind]
        if name in table:
            raise PluginError(RRS_P0103, f"register() registered {kind} {name!r} twice")
        table[name] = implementation

    def find(self, kind: str, name: str) -> object | None:
        """The implementation registered under this name, or None. A missing name is the caller's error: it reports
        its own catalog code (an eval suite naming an unknown metric is an RRS-V code, for example)."""
        return self.table_of(kind).get(name)

    def names(self, kind: str) -> list[str]:
        """The names registered under this kind, sorted."""
        return sorted(self.table_of(kind))

    def table_of(self, kind: str) -> dict[str, object]:
        # Engines pass kinds as constants, so an unknown one here is a bug in the calling code, not a plugin's
        # mistake: ValueError, like any other wrong argument.
        if kind not in self.implementations:
            raise ValueError(f"unknown registry kind {kind!r}; the kinds are {', '.join(REGISTRY_KINDS)}")
        return self.implementations[kind]
