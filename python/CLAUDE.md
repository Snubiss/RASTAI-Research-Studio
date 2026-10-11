# python/: rules for every Python package

Read the root `CLAUDE.md` first. This file adds the Python rules of layout `docs/architecture/layout/python.md` §1.4; each package's own `CLAUDE.md` adds its local ones.

## Where things are

- `pyproject.toml`: the uv workspace root (members: every `rastai*` folder here, never `envs/`) and the ruff and pytest settings for all of `python/`.
- `conftest.py`: shared fixtures (`repo_root`, `fixtures_folder`) and the marker rules: `gpu` tests skip without a device, `posix_only` tests skip on Windows.
- `<package>/`: one import package, flat layout, with `pyproject.toml` (build backend `uv_build`, version = the app version), `CLAUDE.md` and `tests/test_<module>.py`.
- `envs/<backend>/`: one locked environment per backend (`envs/README.md`). Nothing is ever resolved on a user's machine.

## Running

- `just env-cpu` syncs `.dev/envs/cpu` from `envs/cpu/uv.lock`. `just test-python` runs pytest in it; `just fmt-check` and `just lint` run ruff. All of them run inside `just check`.
- Python 3.12 only. Add a package to an environment by pinning it exactly (`==`) in `envs/<backend>/pyproject.toml`, running `uv lock` there, and listing it, with every new transitive package, in `deps/allowlist-python.json` in the same PR. It must be in spec §27 first.

## Hard rules (layout §1.4)

1. **No pickle.** No `pickle`, `torch.load`, `torch.save`, `torch.distributed.checkpoint`, `*_object` collectives or `.bin` weights. ruff bans the first three; `check-no-pickle` greps for the rest.
2. **No runtime installs.** No `pip` or `uv pip` from a running process. Packages arrive only through a locked `uv sync` run by the node.
3. **Workers don't look things up.** No reading `settings.json`, `rastai.db`, `vault.db` or the keychain; settings arrive in `arguments_json`, paths in `job.json`.
4. **BLAKE3, safetensors headers, GGUF bytes and document validation go through `rastai_formats`.** Never re-implement a Rust-owned check.
5. **One error class per package**, carrying `code` (from `rastai_plugin.catalog`) and `message`. Only the edge turns it into a `Diagnostic`.
6. **One wrapper module per external library** (contracts §2.5). Everything else calls the wrapper.
7. **Wire types stay at the edge.** Engines never import `rastai_proto`; they emit `(event_type, dict)`.
8. **One GPU thread per job**; gRPC handler threads only queue requests or flip flags.
9. **Captured code is capture-safe**: no `.item()`, `.cpu()`, `.tolist()` or Python branch on a tensor value inside a captured step.
10. **Deterministic**: never Python's `hash()` or the global `random`; seed a `torch.Generator` or a private `random.Random`.
11. **In-house parsers of untrusted input** get a seeded mutation test marked `fuzz`.

## Style (the owner's)

- Constants at the top of each file, `SCREAMING_SNAKE_CASE`, with "(spike)" and the number when measured.
- Readable names (`embedding_size`, `layer_count`, never `n_embd`), small functions named verb-first, a thin `main`, comments that explain why.
- Data is `@dataclass` (frozen where possible). No metaclasses, no pydantic, no decorators that change behavior except `@dataclass`, `@torch.inference_mode()` and `@torch.no_grad()`.
- Type hints on every public function.
- Names come only from `docs/architecture/contracts.md`; a new shared name goes there in the same PR.
