"""check-names on a fixture pair: a contracts.md excerpt with every section the check reads, and a names.json.

The base pair agrees exactly, so each test changes one thing and sees the one problem it makes (contracts 12.6,
layout rust-control-plane.md 2.8): a name only in code, a name only in contracts, the same name with a milestone
marker, and the ways a marker can or can't cover a name.
"""

import json

CONTRACTS_FILE = "docs/architecture/contracts.md"
NAMES_FILE = "schemas/names.json"

# Every section check-names reads, each with one table that agrees with BASE_NAMES.
BASE_CONTRACTS = """# Shared Contracts

### 2.1 Rust crates

| Crate | Purpose | Process |
|---|---|---|
| `rastai-core` | Command registry | daemon |

### 2.2 Python packages

| Package | Purpose |
|---|---|
| `rastai_worker` | Job entry point |

### 2.3 Apps and JavaScript packages

| Package | Purpose |
|---|---|
| `@rastai/client` | Daemon client |

### 5.2 Document types

| Document | Type / schema id | Container | Main fields (after the header) |
|---|---|---|---|
| **Settings** | `rastai.settings/1` | JSON `settings.json` | the keys in section 11 |
| Intervention set | `InterventionSet` (shared type, no header) | inside other documents | `steering[]` |

### 6.4 Initial command catalog

**Inference**

| Command | Purpose | Scope | Undo | Kind |
|---|---|---|---|---|
| `model.load` | load on a node and GPU | inference.manage | | job |

### 7.3 Event types

| Event type | Class | Emitter | Main payload |
|---|---|---|---|
| `job.state_changed` | durable | node | job_id, from_state, to_state |

## 8. Storage

### 8.2 `rastai.db` (SQLite, WAL, one writer thread)

| Table | Columns |
|---|---|
| `runs` | `run_id`, `archive_state` (`local`, `archived`; 9.9): the run's `notes_column` is prose |

| Relation | `from_id` → `to_id` |
|---|---|
| `branch_of` | a message → its branch |

### 8.3 `vault.db` (SQLite, WAL, one writer thread)

| Table | Columns |
|---|---|
| `conversations` (M3) | `conversation_id`, `title` |

### 10.3 Codes assigned so far

**`RRS-E`**

| Code | Severity | Meaning |
|---|---|---|
| `RRS-E1003` | error | Unknown field |

### 11.2 Top-level keys

| Section | Main keys (defaults) |
|---|---|
| `ui` | `theme` (`system`; also `light`), `density` (`comfortable`) |

### 15.1 Rust names by crate

| Crate | Names |
|---|---|
| `rastai-core` | `ALIASES` (`doctor` → `system.doctor`), `CLI_MODE_WORDS` (`serve`, `node check-update` (L2)) |

### 15.2 Python names by package

| Package | Names |
|---|---|
| `rastai_worker` | `python -m rastai_worker run` |

### 15.3 JavaScript names by package

| Package | Names |
|---|---|
| `@rastai/client` | `DaemonClient` |

### 15.4 Desktop shell, CLI and HTTP details
"""

BASE_NAMES = {
    "format_version": 1,
    "command_ids": ["model.load"],
    "cli_verbs": ["doctor", "model load", "node check-update", "serve"],
    "event_types": ["job.state_changed"],
    "error_codes": ["RRS-E1003"],
    "tables": {"rastai.db": {"runs": ["run_id", "archive_state"]}, "vault.db": {}},
    "settings_keys": ["ui.density", "ui.theme"],
    "document_schema_ids": ["rastai.settings/1"],
    "crates": ["rastai-core"],
    "js_packages": ["@rastai/client"],
    "python_packages": ["rastai_worker"],
}


def run_on(write_tree, run_check, contracts=BASE_CONTRACTS, names=None):
    names = BASE_NAMES if names is None else names
    root = write_tree({CONTRACTS_FILE: contracts, NAMES_FILE: json.dumps(names, indent=2)})
    return run_check("check-names", "--root", str(root))


def names_with(key, value):
    names = json.loads(json.dumps(BASE_NAMES))
    names[key] = value
    return names


def with_row(heading_line, row):
    """BASE_CONTRACTS with one more table row right after the first table row under heading_line."""
    start = BASE_CONTRACTS.index(heading_line)
    divider = BASE_CONTRACTS.index("|---", start)
    first_row_end = BASE_CONTRACTS.index("\n", BASE_CONTRACTS.index("\n", divider) + 1)
    return BASE_CONTRACTS[: first_row_end + 1] + row + "\n" + BASE_CONTRACTS[first_row_end + 1 :]


def problem_lines(result):
    return [line for line in result.stdout.splitlines() if "problem(s)" not in line]


# ---- The four rules (contracts 12.6) ----


def test_a_name_in_both_passes(write_tree, run_check):
    result = run_on(write_tree, run_check)
    assert result.returncode == 0, result.stdout


def test_a_name_only_in_code_fails_as_unregistered(write_tree, run_check):
    names = names_with("event_types", ["job.progress", "job.state_changed"])
    result = run_on(write_tree, run_check, names=names)
    assert result.returncode == 1
    assert problem_lines(result) == [
        "check-names: crates/rastai-core/src/event_types.rs: unregistered name: event type `job.progress` is in the "
        "code (schemas/names.json) but not in contracts.md 7.3; add it there in this PR (a wire, document or command "
        "shape needs its contract PR first)"
    ]


def test_a_name_only_in_contracts_fails_as_stale(write_tree, run_check):
    contracts = with_row("### 7.3 ", "| `job.progress` | ephemeral | worker | fraction, message |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    line = contracts.splitlines().index("| `job.progress` | ephemeral | worker | fraction, message |") + 1
    assert problem_lines(result) == [
        f"check-names: docs/architecture/contracts.md:{line}: stale name: event type `job.progress` is in "
        "contracts.md 7.3 but not in the code (schemas/names.json); remove it, or mark it with the milestone that "
        "builds it, such as (M4)"
    ]


def test_the_same_name_marked_m4_passes(write_tree, run_check):
    contracts = with_row("### 7.3 ", "| `job.progress` (M4) | ephemeral | worker | fraction, message |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 0, result.stdout


def test_every_marker_kind_covers_a_name(write_tree, run_check):
    for marker in ["(M11)", "(L2)", "(Phase 2)", "(Phase 3)"]:
        contracts = with_row("### 6.4 ", f"| `model.unload` {marker} | unload | inference.manage | | sync |")
        result = run_on(write_tree, run_check, contracts=contracts)
        assert result.returncode == 0, (marker, result.stdout)


def test_a_marker_lookalike_covers_nothing(write_tree, run_check):
    # "(M8 or Phase 2)" and "(7, L2)" are prose, not markers: the whole parenthesized text must be one marker.
    for note in ["(M8 or Phase 2)", "(7, L2)", "(m4)"]:
        contracts = with_row("### 6.4 ", f"| `model.unload` {note} | unload | inference.manage | | sync |")
        result = run_on(write_tree, run_check, contracts=contracts)
        assert result.returncode == 1, note
        assert "stale name: command id `model.unload`" in result.stdout


# ---- Where a marker reaches ----


def test_a_marker_after_one_name_covers_only_that_name_in_its_cell(write_tree, run_check):
    contracts = with_row("### 6.4 ", "| `plugin.list` (M1) / `plugin.permissions` (M8) | plugins | read | | sync |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 0, result.stdout
    # A name before the marker without one of its own is covered by the row's marker.
    contracts = with_row("### 6.4 ", "| `token.create` / `token.list` (M1) | tokens | system.admin | | sync |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 0, result.stdout


def test_a_marker_elsewhere_in_the_row_covers_the_row(write_tree, run_check):
    contracts = with_row("### 6.4 ", "| `archive.sync` | sync now (M11) | system.admin | | job |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 0, result.stdout


def test_a_marker_in_the_heading_covers_the_section(write_tree, run_check):
    contracts = with_row("### 7.3 ", "| `job.progress` | ephemeral | worker | fraction |").replace(
        "### 7.3 Event types", "### 7.3 Event types (M2)"
    )
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 0, result.stdout


def test_a_marked_column_does_not_hide_a_dropped_one(write_tree, run_check):
    # In a list cell a marker covers only the name it follows, so `pinned` (M5) can't hide the dropped `state`.
    contracts = BASE_CONTRACTS.replace(
        "| `runs` | `run_id`, `archive_state`", "| `runs` | `run_id`, `state`, `pinned` (M5), `archive_state`"
    )
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    assert "stale name: rastai.db column `runs.state`" in result.stdout
    assert "runs.pinned" not in result.stdout
    assert "1 problem(s)" in result.stdout


def test_a_marker_in_a_list_cells_notes_covers_nothing(write_tree, run_check):
    contracts = BASE_CONTRACTS.replace(
        "| `runs` | `run_id`, `archive_state`", "| `runs` | `run_id`, `state`, `archive_state`"
    ).replace("is prose |", "is prose (M5) |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    assert "stale name: rastai.db column `runs.state`" in result.stdout


# ---- Each kind's own reading ----


def test_tables_and_columns_compare_both_ways(write_tree, run_check):
    names = names_with(
        "tables",
        {
            "rastai.db": {"runs": ["run_id", "archive_state", "kind"], "jobs": ["job_id"]},
            "vault.db": {"conversations": ["conversation_id"]},
        },
    )
    contracts = with_row("### 8.2 ", "| `transfers` | `transfer_id` |")
    result = run_on(write_tree, run_check, contracts=contracts, names=names)
    assert result.returncode == 1
    source = "crates/rastai-store/src/index/migrations/"
    assert f"{source}: unregistered name: rastai.db column `runs.kind`" in result.stdout
    assert f"{source}: unregistered name: rastai.db table `jobs`" in result.stdout
    assert "stale name: rastai.db table `transfers`" in result.stdout
    # The vault's table is marked (M3), so its unbuilt `title` column passes, and its built one is registered.
    assert "conversations" not in result.stdout
    assert "3 problem(s)" in result.stdout


def test_notes_after_a_column_list_are_not_columns(write_tree, run_check):
    # `notes_column` sits after the ':' that starts the cell's notes, and `local` inside parentheses.
    names = names_with("tables", {"rastai.db": {"runs": ["run_id", "archive_state", "notes_column"]}, "vault.db": {}})
    result = run_on(write_tree, run_check, names=names)
    assert result.returncode == 1
    assert "unregistered name: rastai.db column `runs.notes_column`" in result.stdout


def test_settings_keys_are_section_dot_key_and_stop_at_an_object(write_tree, run_check):
    contracts = with_row("### 11.2 ", "| `training` | `lora{rank 16, alpha 32}`, `dev_paths[]` |")
    names = names_with("settings_keys", ["training.dev_paths", "training.lora", "ui.density", "ui.theme"])
    result = run_on(write_tree, run_check, contracts=contracts, names=names)
    assert result.returncode == 0, result.stdout
    names = names_with("settings_keys", ["training.lora.rank", "ui.density", "ui.theme"])
    result = run_on(write_tree, run_check, contracts=contracts, names=names)
    assert result.returncode == 1
    assert "unregistered name: settings key `training.lora.rank`" in result.stdout
    assert "stale name: settings key `training.lora`" in result.stdout
    assert "stale name: settings key `training.dev_paths`" in result.stdout


def test_cli_verbs_are_command_words_or_the_fixed_words(write_tree, run_check):
    contracts = with_row("### 6.4 ", "| `token.login_code.create` (M1) | a login code | system.admin | | sync |")
    verbs = ["doctor", "model load", "node check-update", "serve", "token login-code create"]
    result = run_on(write_tree, run_check, contracts=contracts, names=names_with("cli_verbs", verbs))
    assert result.returncode == 0, result.stdout
    result = run_on(write_tree, run_check, names=names_with("cli_verbs", ["doctor", "serve", "tunnel"]))
    assert result.returncode == 1
    assert problem_lines(result) == [
        "check-names: crates/rastai-core/src/commands/naming.rs (CLI_MODE_WORDS, ALIASES) or a command id: "
        "unregistered name: CLI verb `tunnel` is in the code (schemas/names.json) but not in contracts.md 6.4 or "
        "15.1; add it there in this PR (a wire, document or command shape needs its contract PR first)"
    ]


def test_crates_and_packages_are_read_from_2_and_15(write_tree, run_check):
    # Listed in 2.1 only: registered. Listed in 15.1 without a marker and not built: stale there.
    contracts = with_row("### 2.1 ", "| `rastai-store` | Databases | daemon |")
    contracts = contracts.replace(
        "| `rastai-core` | `ALIASES`", "| `rastai-node` | `NodeOptions` |\n| `rastai-core` | `ALIASES`"
    )
    names = names_with("crates", ["rastai-core", "rastai-store"])
    result = run_on(write_tree, run_check, contracts=contracts, names=names)
    assert result.returncode == 1
    line = contracts.splitlines().index("| `rastai-node` | `NodeOptions` |") + 1
    assert problem_lines(result) == [
        f"check-names: docs/architecture/contracts.md:{line}: stale name: crate `rastai-node` is in contracts.md 15.1 "
        "but not in the code (schemas/names.json); remove it, or mark it with the milestone that builds it, such as "
        "(M4)"
    ]


def test_only_document_schema_ids_count_in_5_2(write_tree, run_check):
    # `InterventionSet` in the same column is a shared type, not a schema id, so it is never stale.
    names = names_with("document_schema_ids", ["rastai.model/1", "rastai.settings/1"])
    result = run_on(write_tree, run_check, names=names)
    assert result.returncode == 1
    assert problem_lines(result) == [
        "check-names: crates/rastai-formats/src/documents/: unregistered name: document schema id `rastai.model/1` is "
        "in the code (schemas/names.json) but not in contracts.md 5.2; add it there in this PR (a wire, document or "
        "command shape needs its contract PR first)"
    ]


# ---- The reader and contracts.md change together ----


def test_a_moved_section_fails(write_tree, run_check):
    contracts = BASE_CONTRACTS.replace("### 7.3 Event types", "### 7.4 Event types")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    assert "no section heading starting `### 7.3`" in result.stdout


def test_a_table_with_another_first_column_fails(write_tree, run_check):
    contracts = BASE_CONTRACTS.replace("| Code | Severity | Meaning |", "| Error | Severity | Meaning |")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    assert "no table whose first column is `Code` under `### 10.3 Codes assigned so far`" in result.stdout


def test_a_missing_names_file_fails(write_tree, run_check):
    root = write_tree({CONTRACTS_FILE: BASE_CONTRACTS})
    result = run_check("check-names", "--root", str(root))
    assert result.returncode == 1
    assert "schemas/names.json: missing; run `just codegen`" in result.stdout


def test_the_real_repository_passes(run_check):
    result = run_check("check-names")
    assert result.returncode == 0, result.stdout


def test_a_moved_command_section_is_reported_once(write_tree, run_check):
    # Both the command ids and the CLI verbs read 6.4; the reader's problem with it is one problem.
    contracts = BASE_CONTRACTS.replace("### 6.4 Initial command catalog", "### 6.5 Initial command catalog")
    result = run_on(write_tree, run_check, contracts=contracts)
    assert result.returncode == 1
    assert "1 problem(s)" in result.stdout
