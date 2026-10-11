"""check-generated on passing and failing fixture trees, with a tiny generator standing in for codegen."""

import sys

# A generator that writes generated/greeting.txt and generated/nested/count.txt under the output root it's given.
GENERATOR_SCRIPT = """import sys
from pathlib import Path

output_root = Path(sys.argv[1])
(output_root / "generated/nested").mkdir(parents=True, exist_ok=True)
(output_root / "generated/greeting.txt").write_text("hello\\n")
(output_root / "generated/nested/count.txt").write_text("2\\n")
"""
FRESH_FILES = {"generated/greeting.txt": "hello\n", "generated/nested/count.txt": "2\n"}


def make_generator(check):
    return check.Generator(
        name="greeting-codegen",
        command=[sys.executable, "tools/greeting_codegen.py", check.OUTPUT_ROOT_PLACEHOLDER],
        outputs=["generated"],
    )


def test_checked_in_output_equal_to_a_fresh_run_passes(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree({"tools/greeting_codegen.py": GENERATOR_SCRIPT} | FRESH_FILES)
    assert check.find_stale_files(root, [make_generator(check)]) == []


def test_hand_edited_missing_and_stale_files_fail(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree(
        {
            "tools/greeting_codegen.py": GENERATOR_SCRIPT,
            "generated/greeting.txt": "hello, edited by hand\n",
            "generated/old.txt": "codegen stopped writing this\n",
        }
    )
    problems = check.find_stale_files(root, [make_generator(check)])
    assert problems == [
        "generated/nested/count.txt: missing; run `just codegen` and commit the result",
        "generated/old.txt: codegen no longer writes it; delete it",
        "generated/greeting.txt: differs from a fresh codegen run; never edit it, run `just codegen`",
    ]


def test_a_failing_generator_fails(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree({"tools/greeting_codegen.py": "import sys\nsys.exit('type description is broken')\n"})
    problems = check.find_stale_files(root, [make_generator(check)])
    assert problems[0] == "greeting-codegen failed (exit 1):"
    assert "type description is broken" in problems


# A generator that writes one of its two files and then fails, the way a generator dies partway through.
PARTLY_WRITING_FAILING_SCRIPT = """import sys
from pathlib import Path

output_root = Path(sys.argv[1])
(output_root / "generated").mkdir(parents=True, exist_ok=True)
(output_root / "generated/greeting.txt").write_text("hello\\n")
sys.exit("error: expected one of `!` or `::`, found `<eof>`")
"""


def test_a_failing_generator_reports_only_its_error_and_never_says_delete(write_tree, load_check):
    # The checked-in outputs are present (and one is even hand-edited). The generator fails, so its temporary tree
    # is empty: comparing against it would call every checked-in file stale and tell the reader to delete it.
    check = load_check("check-generated")
    root = write_tree(
        {
            "tools/greeting_codegen.py": "import sys\nsys.exit('error: expected one of `!` or `::`, found `<eof>`')\n",
            "generated/greeting.txt": "hello, edited by hand\n",
            "generated/nested/count.txt": "2\n",
        }
    )
    problems = check.find_stale_files(root, [make_generator(check)])
    assert problems == [
        "greeting-codegen failed (exit 1):",
        "error: expected one of `!` or `::`, found `<eof>`",
    ]


def test_a_generator_that_fails_partway_is_not_compared(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree({"tools/greeting_codegen.py": PARTLY_WRITING_FAILING_SCRIPT} | FRESH_FILES)
    problems = check.find_stale_files(root, [make_generator(check)])
    assert problems == [
        "greeting-codegen failed (exit 1):",
        "error: expected one of `!` or `::`, found `<eof>`",
    ]


def test_a_failing_generator_prints_its_recovery_hint(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree({"tools/broken_codegen.py": "import sys\nsys.exit('broken')\n"})
    generator = check.Generator(
        name="broken-codegen",
        command=[sys.executable, "tools/broken_codegen.py", check.OUTPUT_ROOT_PLACEHOLDER],
        outputs=["generated"],
        failure_hint="see the recovery note",
    )
    assert check.find_stale_files(root, [generator]) == [
        "broken-codegen failed (exit 1):",
        "broken",
        "broken-codegen: see the recovery note",
    ]


def test_a_missing_generator_tool_is_named(write_tree, load_check):
    check = load_check("check-generated")
    generator = check.Generator(name="buf generate", command=["rastai-no-such-tool", "generate"], outputs=["generated"])
    problems = check.find_stale_files(write_tree({}), [generator])
    assert problems[0] == (
        "buf generate: `rastai-no-such-tool` isn't installed; the justfile header says how to install it"
    )


def test_ignored_folders_inside_an_output_are_not_compared(write_tree, load_check):
    # Importing generated Python writes __pycache__/ beside it; git ignores it and so does the comparison.
    check = load_check("check-generated")
    root = write_tree(
        {"tools/greeting_codegen.py": GENERATOR_SCRIPT, "generated/__pycache__/greeting.cpython-312.pyc": "bytes"}
        | FRESH_FILES
    )
    assert check.find_stale_files(root, [make_generator(check)]) == []


def test_buf_writes_the_three_proto_outputs(load_check):
    check = load_check("check-generated")
    buf = check.GENERATORS[0]
    assert buf.command[:2] == ["buf", "generate"]
    assert check.OUTPUT_ROOT_PLACEHOLDER in buf.command
    assert buf.outputs == [
        "crates/rastai-proto/src/generated",
        "python/rastai_proto/rastai_proto",
        "packages/proto/src/generated",
    ]


# A generator that owns one file, generated/notes/errors.md, in a folder that also holds a hand-written file.
SINGLE_FILE_GENERATOR_SCRIPT = """import sys
from pathlib import Path

output_root = Path(sys.argv[1])
(output_root / "generated/notes").mkdir(parents=True, exist_ok=True)
(output_root / "generated/notes/errors.md").write_text("# Errors\\n")
"""


def make_single_file_generator(check):
    return check.Generator(
        name="errors-page-codegen",
        command=[sys.executable, "tools/errors_page_codegen.py", check.OUTPUT_ROOT_PLACEHOLDER],
        outputs=["generated/notes/errors.md"],
    )


def test_a_single_file_output_is_compared_alone(write_tree, load_check):
    # The hand-written file beside it is neither missing from the fresh run nor stale.
    check = load_check("check-generated")
    root = write_tree(
        {
            "tools/errors_page_codegen.py": SINGLE_FILE_GENERATOR_SCRIPT,
            "generated/notes/errors.md": "# Errors\n",
            "generated/notes/README.md": "written by hand\n",
        }
    )
    assert check.find_stale_files(root, [make_single_file_generator(check)]) == []


def test_a_hand_edited_single_file_output_fails(write_tree, load_check):
    check = load_check("check-generated")
    edited = write_tree(
        {"tools/errors_page_codegen.py": SINGLE_FILE_GENERATOR_SCRIPT, "generated/notes/errors.md": "# Edited\n"}
    )
    assert check.find_stale_files(edited, [make_single_file_generator(check)]) == [
        "generated/notes/errors.md: differs from a fresh codegen run; never edit it, run `just codegen`"
    ]


def test_a_missing_single_file_output_fails(write_tree, load_check):
    check = load_check("check-generated")
    missing = write_tree({"tools/errors_page_codegen.py": SINGLE_FILE_GENERATOR_SCRIPT})
    assert check.find_stale_files(missing, [make_single_file_generator(check)]) == [
        "generated/notes/errors.md: missing; run `just codegen` and commit the result"
    ]


def test_a_single_file_output_codegen_stopped_writing_fails(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree({"tools/quiet_codegen.py": "", "generated/notes/errors.md": "# Errors\n"})
    generator = check.Generator(
        name="quiet-codegen",
        command=[sys.executable, "tools/quiet_codegen.py", check.OUTPUT_ROOT_PLACEHOLDER],
        outputs=["generated/notes/errors.md"],
    )
    assert check.find_stale_files(root, [generator]) == [
        "generated/notes/errors.md: codegen no longer writes it; delete it"
    ]


def test_the_catalog_generator_writes_its_four_outputs_after_buf(load_check):
    check = load_check("check-generated")
    catalog = check.GENERATORS[1]
    assert catalog.command[:2] == ["cargo", "run"]
    assert "rastai-catalog-codegen" in catalog.command
    assert catalog.command[-2:] == ["--output", check.OUTPUT_ROOT_PLACEHOLDER]
    assert catalog.outputs == [
        "crates/rastai-diag/src/generated",
        "packages/catalog/src/generated",
        "python/rastai_plugin/rastai_plugin/catalog/generated.py",
        "docs/reference/errors.md",
    ]
    # rastai-codegen builds on rastai-diag, so its failure points at the documented way out.
    assert "crates/rastai-diag/CLAUDE.md" in catalog.failure_hint


def test_the_formats_generator_writes_its_five_outputs_after_the_catalog(load_check):
    check = load_check("check-generated")
    formats = check.GENERATORS[2]
    assert formats.command[:2] == ["cargo", "run"]
    assert "rastai-formats-codegen" in formats.command
    assert formats.command[-2:] == ["--output", check.OUTPUT_ROOT_PLACEHOLDER]
    assert formats.outputs == [
        "schemas",
        "packages/formats/src/generated",
        "packages/client/src/generated/commands.js",
        "python/rastai_formats/rastai_formats/generated",
        "docs/reference/commands.md",
    ]


def test_a_failing_generator_does_not_stop_the_next_one_being_compared(write_tree, load_check):
    check = load_check("check-generated")
    root = write_tree(
        {
            "tools/broken_codegen.py": "import sys\nsys.exit('broken')\n",
            "tools/errors_page_codegen.py": SINGLE_FILE_GENERATOR_SCRIPT,
            "generated/notes/errors.md": "# Edited\n",
        }
    )
    broken = check.Generator(
        name="broken-codegen",
        command=[sys.executable, "tools/broken_codegen.py", check.OUTPUT_ROOT_PLACEHOLDER],
        outputs=["generated/broken"],
    )
    problems = check.find_stale_files(root, [broken, make_single_file_generator(check)])
    assert problems == [
        "broken-codegen failed (exit 1):",
        "broken",
        "generated/notes/errors.md: differs from a fresh codegen run; never edit it, run `just codegen`",
    ]
