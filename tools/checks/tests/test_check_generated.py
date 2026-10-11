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
