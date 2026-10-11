"""The source scanners in repo_files.py that several checks share: what counts as code, string and comment."""

import sys
from pathlib import Path

import pytest

# repo_files.py sits one folder up, beside the check scripts that import it.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from repo_files import rust_code_only, split_javascript  # noqa: E402


@pytest.mark.parametrize(
    ("source", "non_code_part"),
    [
        ('call("http://x", unsafe { g() });', '"http://x"'),
        ("a(); // unsafe { }", "// unsafe { }"),
        ("/* outer /* inner */ unsafe { */ b();", "/* outer /* inner */ unsafe { */"),
        ('let s = r#"say "hi" // not a comment"#; c();', 'r#"say "hi" // not a comment"#'),
        ("let q = '\"'; d();", "'\"'"),
        ('let s = "line one\nline two"; e();', '"line one\nline two"'),
    ],
)
def test_rust_code_only_blanks_strings_and_comments(source, non_code_part):
    """The comment or string becomes spaces (its line breaks stay); the code around it is unchanged."""
    blanked = "".join("\n" if character == "\n" else " " for character in non_code_part)
    assert rust_code_only(source) == source.replace(non_code_part, blanked)


def test_lifetimes_stay_code():
    source = "fn f<'a>(x: &'a str) -> &'a str { x }"
    assert rust_code_only(source) == source


def test_rust_code_only_keeps_offsets(source='x = "a\\"b"; /* c */ y'):
    assert len(rust_code_only(source)) == len(source)


def test_an_apostrophe_in_jsx_text_starts_no_string():
    source = "<p>Don't reopen issue #123</p>"
    assert [kind for kind, _, _ in split_javascript(source)] == ["code"]


def test_a_tagged_template_is_still_a_string():
    source = "css`color: red;`"
    assert [kind for kind, _, _ in split_javascript(source)] == ["code", "string"]
