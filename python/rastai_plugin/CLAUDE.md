# python/rastai_plugin: the Python plugin SDK

Read the root `CLAUDE.md` and `python/CLAUDE.md` first. The layout is `docs/architecture/layout/frontend-and-plugins.md` §11 (and `docs/architecture/layout/python.md` §1.6 for how engines use it).

- **A skeleton for now.** M1-12 landed the package and its generated catalog; M1-37 adds the registry, the base classes, the module loader and the host client.
- **Low in the stack.** Engines define their extension points on this package, so it never imports an engine or a plugin (contracts 2.4). Its only internal imports are `rastai_proto` and `rastai_formats` (`check-imports`).
- **Base classes stay small:** a few methods each, plain dataclasses for data. No pickle.
- **`rastai_plugin/catalog/generated.py` is generated** by `rastai-catalog-codegen` from `catalog/` and the first-party plugin catalogs, and never edited by hand: change the catalog JSON and run `just codegen`. `just codegen-check` fails when it differs from a fresh run. ruff skips it.
- Tests are in `tests/test_<module>.py`.
