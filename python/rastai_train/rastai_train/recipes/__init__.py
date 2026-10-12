"""The first-party recipes (layout python.md section 5.1). Each is registered by name, like a plugin's, so the train
job finds every recipe through the one registry (plugins layout 11.2). The other recipes join in their build steps
(section 5.9)."""

from rastai_plugin import PluginRegistry
from rastai_train.recipes.classification import ClassificationRecipe

RECIPES_KIND = "recipes"
# Recipe name -> its class. The train job makes one instance per run.
FIRST_PARTY_RECIPES = {
    ClassificationRecipe.name: ClassificationRecipe,
}


def register_first_party(registry: PluginRegistry) -> None:
    """Registers every first-party recipe class under its name."""
    for name, recipe_class in FIRST_PARTY_RECIPES.items():
        registry.register(RECIPES_KIND, name, recipe_class)
