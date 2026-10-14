"""The source scanners in repo_files.py that several checks share: what counts as code, string and comment."""

import sys
from pathlib import Path

import pytest

# repo_files.py sits one folder up, beside the check scripts that import it.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from repo_files import javascript_code_only, rust_code_only, split_javascript  # noqa: E402


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


@pytest.mark.parametrize(
    ("source", "non_code_part"),
    [
        ('f("import.meta.glob(x)");', '"import.meta.glob(x)"'),
        ("a(); // base: 1", "// base: 1"),
        ("b(/* base */ 1);", "/* base */"),
        ("c(`x${y}`);", "`x${y}`"),
    ],
)
def test_javascript_code_only_blanks_strings_and_comments(source, non_code_part):
    """A glob call or a base key written in a string or a comment is not code, so the glob check can't see it there."""
    blanked = "".join("\n" if character == "\n" else " " for character in non_code_part)
    assert javascript_code_only(source) == source.replace(non_code_part, blanked)


def test_rust_code_only_keeps_offsets(source='x = "a\\"b"; /* c */ y'):
    assert len(rust_code_only(source)) == len(source)


def test_an_apostrophe_in_jsx_text_starts_no_string():
    source = "<p>Don't reopen issue #123</p>"
    assert [kind for kind, _, _ in split_javascript(source)] == ["code"]


def test_a_tagged_template_is_still_a_string():
    source = "css`color: red;`"
    assert [kind for kind, _, _ in split_javascript(source)] == ["code", "string"]


def test_a_quote_right_after_from_or_import_starts_a_string():
    """Minified and prettier-ignored code writes no space before the specifier (FU-276 item 1)."""
    source = 'import{X}from"@rastai/ui/security";import"a";'
    strings = [source[start:end] for kind, start, end in split_javascript(source) if kind == "string"]
    assert strings == ['"@rastai/ui/security"', '"a"']


def test_a_quote_after_a_longer_word_stays_jsx_text():
    """reimport' and Let's are words that end in the keywords' letters or hold an apostrophe: still text."""
    source = "<p>Let's reimport'</p>"
    assert [kind for kind, _, _ in split_javascript(source)] == ["code"]
