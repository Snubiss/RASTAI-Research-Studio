//! The catalog rules (contracts 10; layout rust-control-plane.md 2.6 and 3.5): every code is well formed, unique,
//! inside a range its owner holds and in its owner's file, and never deleted once it reached the base branch (a code
//! that is no longer raised is marked `retired` instead, so its number is never reused). Every docs anchor is a
//! lowercase kebab-case slug, used once among the errors and once among the terms, and every term id is unique.
//!
//! The owners' ranges in src/catalog_ranges.rs are a hand copy of the contracts 10.1 table, so one test reads that
//! table and compares the two: a prefix, an area or a range that only one of them has fails it, and neither can
//! drift from the other unnoticed.
//!
//! The catalog and contracts 10.3 hold the same codes with the same severities (M1-13): a code is listed there first,
//! and a row marked for a later milestone ("(M4)", the plugins' catalogs) may wait for its entry. The source scan
//! finds every code that first-party source names outside its tests and checks that the catalog has it, and the
//! placeholder scan checks that a `diagnostic("RRS-...", &[("name", ...)])` call and its entry name the same values
//! (layout 2.6, M1-13). One rule joins later, with its input: `fix_command_id` naming a real command (M1-22, with the
//! first rows of the command table).
//!
//! Each rule is a small function that returns its problems. The tests run each rule on the real catalog, and on a
//! small sample that breaks it, so a rule that stops catching anything fails too.
//!
//! The merge-base test reads git history, which a shallow CI checkout doesn't have, so it is `#[ignore]`d in a plain
//! `cargo test` and run by `just codegen-check` (inside `just check`), next to `proto-check`'s own merge-base step.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::process::Command;

use rastai_codegen::catalog_ranges::{CATALOG_PREFIXES, CatalogPrefix, CodeRange, find_prefix, find_range, split_code};
use rastai_codegen::catalog_source::{ErrorRecord, TermRecord, read_catalog, read_error_codes};
use rastai_codegen::literals::severity_word;
use rastai_codegen::repo_root::find_repo_root;
use rastai_diag::diagnostic::Severity;

/// The branch a pull request merges into; CI passes the PR's base, as for check-schema-diff (contracts 12.6).
const BASE_REF_VARIABLE: &str = "RASTAI_BASE_REF";
const DEFAULT_BASE_REF: &str = "origin/main";
/// The RRS-E file the samples below use; every catalog rule test needs one owner's file.
const CORE_E_FILE: &str = "catalog/errors/RRS-E.json";

/// The table src/catalog_ranges.rs copies: the first table after this heading in contracts.md.
const CONTRACTS_FILE: &str = "docs/architecture/contracts.md";
const RANGES_HEADING: &str = "### 10.1 Ranges";
/// The table's columns are Prefix, Area, Owner and Ranges; its first two lines are the header and the divider.
const PREFIX_COLUMN: usize = 0;
const AREA_COLUMN: usize = 1;
const RANGES_COLUMN: usize = 3;
const TABLE_HEADER_LINES: usize = 2;
/// A Ranges cell reads "0001–0999 syntax · 1000–1999 structure": ranges apart by a middle dot, ends by an en dash.
const RANGE_SEPARATOR: &str = " · ";
const RANGE_DASH: char = '–';
/// A range's area in contracts may end in a note in brackets ("compiler (0600–0699 Eject and ONNX)"), which
/// catalog_ranges.rs keeps as a comment or leaves out.
const AREA_NOTE_START: &str = " (";

/// The code tables of contracts 10.3: every table line from this heading to the next section that starts with a
/// code. Their columns are Code (with an optional milestone marker after it), Severity and Meaning.
const CODES_HEADING: &str = "### 10.3 Codes assigned so far";
const NEXT_SECTION_START: &str = "\n## ";
const CODE_ROW_START: &str = "| `RRS-";
const CODE_COLUMN: usize = 0;
const SEVERITY_COLUMN: usize = 1;

/// The source scan (layout rust-control-plane.md 3.2: "it scans the source for RRS- literals and checks each against
/// the catalog") reads these folders, relative to the repository root, and these kinds of file.
const SCANNED_FOLDERS: [&str; 7] = ["crates", "tools", "python", "packages", "apps", "plugins", "proto"];
const SCANNED_EXTENSIONS: [&str; 5] = [".rs", ".py", ".js", ".jsx", ".proto"];
/// Never entered: build output and installed packages (large, not ours), and test folders, whose files name made-up
/// codes on purpose (an unknown code, a code outside every range).
const SKIPPED_FOLDERS: [&str; 6] = ["target", "node_modules", ".venv", "__pycache__", "dist", "tests"];
/// Test files that sit beside the code instead of in a tests folder.
const PYTHON_TEST_FILE_START: &str = "test_";
const PYTHON_TEST_SETUP_FILE: &str = "conftest.py";
const JAVASCRIPT_TEST_FILE_ENDS: [&str; 2] = [".test.js", ".test.jsx"];
const RUST_EXTENSION: &str = ".rs";
/// A Rust file's unit-test module is an inline `mod name {` under this attribute; the scans read only what comes
/// before it. The same attribute on a helper function or on `mod name;` doesn't end the scan.
const RUST_TEST_ATTRIBUTE: &str = "#[cfg(test)]";
const RUST_ATTRIBUTE_START: &str = "#[";
const RUST_COMMENT_START: &str = "//";
const RUST_MODULE_START: &str = "mod ";
/// A code is "RRS-", one capital letter and four digits: nine characters.
const CODE_START: &str = "RRS-";
const CODE_LENGTH: usize = 9;
/// rastai-diag's two builders, and rastai-formats' migrate wrapper `version_diagnostic`, whose first two arguments
/// are also the code and the values; the placeholder scan reads their calls.
const DIAGNOSTIC_FUNCTIONS: [&str; 3] = ["diagnostic_at", "diagnostic", "version_diagnostic"];
/// Values written inline (`&[("name", x)]`), or passed as a variable (`&values`) declared as an array above the call.
const VALUES_ARRAY_START: &str = "&[";
const VALUES_REFERENCE_START: char = '&';
const ARRAY_START: char = '[';

/// A small contracts 10.3: two core rows and a plugin row marked for a later milestone.
const SAMPLE_CODES_SECTION: &str = "### 10.3 Codes assigned so far (M1)

**`RRS-E`**

| Code | Severity | Meaning |
|---|---|---|
| `RRS-E0001` | error | JSON syntax error at a line and column |
| `RRS-E4001` | info | File from a newer schema version; opened read-only |

**`RRS-L`**

| Code | Severity | Meaning |
|---|---|---|
| `RRS-L0001` (M4) | error | Replay weights not present (fix `model.download`) |

---

## 11. Configuration

| `RRS-E9999` | error | not a code row: it is past the section |
";

/// A small contracts 10.1 and the catalog_ranges.rs rows that match it, for the tests that break one of the two.
const SAMPLE_CONTRACTS: &str = "## 10. Error Catalog

### 10.1 Ranges

| Prefix | Area | Owner | Ranges |
|---|---|---|---|
| `RRS-E` | Formats and validation | `rastai-formats` | 0001–0999 syntax · 1000–1999 structure |
| `RRS-U` | UI client and desktop | `apps/web` | 0001–0099 connection and session · 1000–1999 rendering (WebGL2 unavailable, etc.) |

Rules:
- Every user-facing error has a code.
";
const SAMPLE_PREFIXES: &[CatalogPrefix] = &[
    CatalogPrefix {
        prefix: "RRS-E",
        area: "Formats and validation",
        source_file: "catalog/errors/RRS-E.json",
        ranges: &[
            CodeRange {
                first: 1,
                last: 999,
                area: "syntax",
            },
            CodeRange {
                first: 1000,
                last: 1999,
                area: "structure",
            },
        ],
    },
    CatalogPrefix {
        prefix: "RRS-U",
        area: "UI client and desktop",
        source_file: "catalog/errors/RRS-U.json",
        ranges: &[
            CodeRange {
                first: 1,
                last: 99,
                area: "connection and session",
            },
            CodeRange {
                first: 1000,
                last: 1999,
                area: "rendering",
            },
        ],
    },
];

// ---- The rules ----

/// Codes that aren't "RRS-", one capital letter and four digits.
fn malformed_codes(errors: &[ErrorRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for record in errors {
        if split_code(&record.code).is_none() {
            problems.push(format!(
                "{} ({}): not RRS-<capital letter><4 digits>",
                record.code, record.source_file
            ));
        }
    }
    problems
}

/// Codes listed more than once, in any files.
fn duplicate_codes(errors: &[ErrorRecord]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut problems = Vec::new();
    for record in errors {
        if !seen.insert(record.code.as_str()) {
            problems.push(format!(
                "{} ({}): listed more than once",
                record.code, record.source_file
            ));
        }
    }
    problems
}

/// Codes outside every range of their prefix (contracts 10.1), with a prefix nobody owns, or in another owner's file.
fn codes_outside_their_owner_range(errors: &[ErrorRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for record in errors {
        let Some((prefix, number)) = split_code(&record.code) else {
            continue; // malformed_codes reports it
        };
        let place = format!("{} ({})", record.code, record.source_file);
        match find_prefix(prefix) {
            None => problems.push(format!("{place}: no owner holds the prefix {prefix} (contracts 10.1)")),
            Some(row) if row.source_file != record.source_file => {
                problems.push(format!("{place}: {prefix} codes belong in {}", row.source_file));
            }
            Some(row) if find_range(row, number).is_none() => {
                problems.push(format!(
                    "{place}: {number:04} is outside every {prefix} range (contracts 10.1)"
                ));
            }
            Some(_) => {}
        }
    }
    problems
}

/// Codes the base branch has that the catalog no longer lists. Deleting or renumbering a code is refused: mark it
/// `retired` instead.
fn deleted_codes(base_codes: &BTreeSet<String>, current_codes: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    for code in base_codes.difference(current_codes) {
        problems.push(format!(
            "{code}: deleted or renumbered since the base branch; mark it retired instead"
        ));
    }
    problems
}

/// One docs anchor and where it came from. Errors and terms share the anchor rules.
struct AnchorUse {
    anchor: String,
    place: String,
}

fn error_anchors(errors: &[ErrorRecord]) -> Vec<AnchorUse> {
    errors
        .iter()
        .map(|record| AnchorUse {
            anchor: record.docs_anchor.clone(),
            place: format!("{} ({})", record.code, record.source_file),
        })
        .collect()
}

fn term_anchors(terms: &[TermRecord]) -> Vec<AnchorUse> {
    terms
        .iter()
        .map(|record| AnchorUse {
            anchor: record.docs_anchor.clone(),
            place: format!("term {} ({})", record.term_id, record.source_file),
        })
        .collect()
}

/// Anchors that aren't a lowercase kebab-case slug ("storage-disk-full"). An anchor becomes an HTML id in the
/// reference docs and the end of a fix link, so it must need no escaping anywhere. An empty anchor means "no
/// anchor" and is skipped.
fn malformed_anchors(uses: &[AnchorUse]) -> Vec<String> {
    let mut problems = Vec::new();
    for anchor_use in uses {
        if !anchor_use.anchor.is_empty() && !is_kebab_case_slug(&anchor_use.anchor) {
            problems.push(format!(
                "{}: docs_anchor {:?} is not a lowercase kebab-case slug like \"storage-disk-full\"",
                anchor_use.place, anchor_use.anchor
            ));
        }
    }
    problems
}

/// Anchors used more than once in one list: a link to a shared anchor would land on whichever entry comes first.
fn duplicate_anchors(uses: &[AnchorUse]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut problems = Vec::new();
    for anchor_use in uses {
        if !anchor_use.anchor.is_empty() && !seen.insert(anchor_use.anchor.as_str()) {
            problems.push(format!(
                "{}: docs_anchor {:?} is used more than once",
                anchor_use.place, anchor_use.anchor
            ));
        }
    }
    problems
}

/// Term ids listed more than once, in any files.
fn duplicate_term_ids(terms: &[TermRecord]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut problems = Vec::new();
    for record in terms {
        if !seen.insert(record.term_id.as_str()) {
            problems.push(format!(
                "term {} ({}): listed more than once",
                record.term_id, record.source_file
            ));
        }
    }
    problems
}

/// Lowercase letters and digits in words joined by single hyphens. An empty word means a leading, trailing or
/// doubled hyphen (or an empty text), so every word must have at least one character.
fn is_kebab_case_slug(text: &str) -> bool {
    text.split('-').all(is_slug_word)
}

fn is_slug_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit())
}

// ---- The real catalog ----

fn real_errors() -> Vec<ErrorRecord> {
    read_catalog(&find_repo_root().unwrap())
        .expect("the catalog reads")
        .errors
}

#[test]
fn real_catalog_codes_are_well_formed() {
    assert_eq!(malformed_codes(&real_errors()), Vec::<String>::new());
}

#[test]
fn real_catalog_codes_are_unique() {
    assert_eq!(duplicate_codes(&real_errors()), Vec::<String>::new());
}

#[test]
fn real_catalog_codes_are_inside_their_owner_range() {
    assert_eq!(codes_outside_their_owner_range(&real_errors()), Vec::<String>::new());
}

fn real_terms() -> Vec<TermRecord> {
    read_catalog(&find_repo_root().unwrap())
        .expect("the catalog reads")
        .terms
}

#[test]
fn real_catalog_anchors_are_slugs_used_once() {
    let errors = error_anchors(&real_errors());
    let terms = term_anchors(&real_terms());
    let mut problems = malformed_anchors(&errors);
    problems.extend(malformed_anchors(&terms));
    problems.extend(duplicate_anchors(&errors));
    problems.extend(duplicate_anchors(&terms));
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn real_catalog_term_ids_are_unique() {
    assert_eq!(duplicate_term_ids(&real_terms()), Vec::<String>::new());
}

#[test]
#[ignore = "needs the base branch's git history; `just codegen-check` runs it"]
fn no_code_was_deleted_since_the_merge_base() {
    let repo_root = find_repo_root().unwrap();
    let base_ref = std::env::var(BASE_REF_VARIABLE).unwrap_or_else(|_| DEFAULT_BASE_REF.to_string());
    let merge_base = run_git(&["merge-base", "HEAD", &base_ref]).trim().to_string();
    let mut base_codes = BTreeSet::new();
    for source_file in error_files_at(&merge_base) {
        let text = run_git(&["show", &format!("{merge_base}:{source_file}")]);
        // Only the codes: the base branch's entries may have other fields than today's.
        let codes = read_error_codes(text.as_bytes(), &source_file).expect("the base branch's catalog reads");
        base_codes.extend(codes);
    }
    let current_errors = read_catalog(&repo_root).expect("the catalog reads").errors;
    let current_codes: BTreeSet<String> = current_errors.into_iter().map(|record| record.code).collect();
    assert_eq!(deleted_codes(&base_codes, &current_codes), Vec::<String>::new());
}

/// The catalog error files a commit holds, by the same rule as the reader: catalog/errors/*.json and
/// plugins/<name>/catalog/errors.json.
fn error_files_at(commit: &str) -> Vec<String> {
    let listing = run_git(&[
        "ls-tree",
        "-r",
        "--name-only",
        commit,
        "--",
        "catalog/errors",
        "plugins",
    ]);
    listing
        .lines()
        .filter(|path| is_error_file(path))
        .map(str::to_string)
        .collect()
}

fn is_error_file(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["catalog", "errors", name] => name.ends_with(".json"),
        ["plugins", _, "catalog", "errors.json"] => true,
        _ => false,
    }
}

/// Runs git in the repository and returns its stdout. Any failure fails the test with git's own message, so a
/// missing base branch says what to fetch.
fn run_git(arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(find_repo_root().unwrap())
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {} failed: {}\nFetch the base branch, or set {BASE_REF_VARIABLE} to it.",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git prints UTF-8")
}

// ---- Each rule catches what it should ----

fn sample(code: &str, source_file: &str) -> ErrorRecord {
    ErrorRecord {
        code: code.to_string(),
        severity: Severity::Error,
        title: "A title".to_string(),
        message: "A message.".to_string(),
        basic_text: "Plain words.".to_string(),
        fix_text: String::new(),
        fix_command_id: String::new(),
        docs_anchor: String::new(),
        retired: false,
        source_file: source_file.to_string(),
    }
}

#[test]
fn malformed_codes_are_caught() {
    let errors = [
        sample("RRS-E0001", CORE_E_FILE),
        sample("RRS-E001", CORE_E_FILE),
        sample("RRS-e0002", CORE_E_FILE),
        sample("E0003", CORE_E_FILE),
    ];
    assert_eq!(
        malformed_codes(&errors),
        [
            "RRS-E001 (catalog/errors/RRS-E.json): not RRS-<capital letter><4 digits>",
            "RRS-e0002 (catalog/errors/RRS-E.json): not RRS-<capital letter><4 digits>",
            "E0003 (catalog/errors/RRS-E.json): not RRS-<capital letter><4 digits>",
        ]
    );
}

#[test]
fn a_code_listed_twice_is_caught_even_across_files() {
    let errors = [
        sample("RRS-E0001", CORE_E_FILE),
        sample("RRS-E0002", CORE_E_FILE),
        sample("RRS-E0001", "catalog/errors/RRS-P.json"),
    ];
    assert_eq!(
        duplicate_codes(&errors),
        ["RRS-E0001 (catalog/errors/RRS-P.json): listed more than once"]
    );
}

#[test]
fn codes_outside_their_owner_are_caught() {
    let errors = [
        sample("RRS-E7999", CORE_E_FILE),
        sample("RRS-E8000", CORE_E_FILE),
        sample("RRS-C0999", "catalog/errors/RRS-C.json"),
        sample("RRS-E0000", CORE_E_FILE),
        sample("RRS-Z0001", "catalog/errors/RRS-Z.json"),
        sample("RRS-S0001", CORE_E_FILE),
        sample("RRS-L0001", "plugins/psychology/catalog/errors.json"),
        sample("RRS-L0002", "catalog/errors/RRS-L.json"),
    ];
    assert_eq!(
        codes_outside_their_owner_range(&errors),
        [
            "RRS-E8000 (catalog/errors/RRS-E.json): 8000 is outside every RRS-E range (contracts 10.1)",
            "RRS-C0999 (catalog/errors/RRS-C.json): 0999 is outside every RRS-C range (contracts 10.1)",
            "RRS-E0000 (catalog/errors/RRS-E.json): 0000 is outside every RRS-E range (contracts 10.1)",
            "RRS-Z0001 (catalog/errors/RRS-Z.json): no owner holds the prefix RRS-Z (contracts 10.1)",
            "RRS-S0001 (catalog/errors/RRS-E.json): RRS-S codes belong in catalog/errors/RRS-S.json",
            "RRS-L0002 (catalog/errors/RRS-L.json): RRS-L codes belong in plugins/psychology/catalog/errors.json",
        ]
    );
}

#[test]
fn a_deleted_or_renumbered_code_is_caught_and_a_retired_one_is_not() {
    let base: BTreeSet<String> = ["RRS-E0001", "RRS-E0002", "RRS-E0003"].map(String::from).into();
    // E0001 is still listed (perhaps retired), E0002 was renumbered to E0004, E0003 was deleted.
    let current: BTreeSet<String> = ["RRS-E0001", "RRS-E0004"].map(String::from).into();
    assert_eq!(
        deleted_codes(&base, &current),
        [
            "RRS-E0002: deleted or renumbered since the base branch; mark it retired instead",
            "RRS-E0003: deleted or renumbered since the base branch; mark it retired instead",
        ]
    );
    assert_eq!(deleted_codes(&base, &base), Vec::<String>::new());
}

fn sample_term(term_id: &str, docs_anchor: &str) -> TermRecord {
    TermRecord {
        term_id: term_id.to_string(),
        title: "A title".to_string(),
        basic_text: "Plain words.".to_string(),
        advanced_text: "Exact words.".to_string(),
        docs_anchor: docs_anchor.to_string(),
        source_file: "catalog/terms/terms.json".to_string(),
    }
}

fn sample_with_anchor(code: &str, docs_anchor: &str) -> ErrorRecord {
    let mut record = sample(code, CORE_E_FILE);
    record.docs_anchor = docs_anchor.to_string();
    record
}

#[test]
fn anchors_that_are_not_slugs_are_caught() {
    let errors = [
        sample_with_anchor("RRS-E0001", "storage-disk-full"),
        sample_with_anchor("RRS-E0002", ""),
        sample_with_anchor("RRS-E0003", "format-v2"),
        sample_with_anchor("RRS-E0004", "say\"hi"),
        sample_with_anchor("RRS-E0005", "Disk-Full"),
        sample_with_anchor("RRS-E0006", "disk--full"),
        sample_with_anchor("RRS-E0007", "-disk"),
        sample_with_anchor("RRS-E0008", "disk_full"),
    ];
    let problems = malformed_anchors(&error_anchors(&errors));
    let caught = ["RRS-E0004", "RRS-E0005", "RRS-E0006", "RRS-E0007", "RRS-E0008"];
    assert_eq!(problems.len(), caught.len(), "{problems:#?}");
    for (problem, code) in problems.iter().zip(caught) {
        assert!(problem.starts_with(code), "{problem}");
    }
    assert_eq!(
        problems[0],
        "RRS-E0004 (catalog/errors/RRS-E.json): docs_anchor \"say\\\"hi\" is not a lowercase kebab-case slug like \"storage-disk-full\""
    );
    let terms = [sample_term("logit_lens", "Logit Lens")];
    assert_eq!(malformed_anchors(&term_anchors(&terms)).len(), 1);
}

#[test]
fn an_anchor_used_twice_is_caught_but_empty_anchors_are_not() {
    let errors = [
        sample_with_anchor("RRS-E0001", "disk-full"),
        sample_with_anchor("RRS-E0002", ""),
        sample_with_anchor("RRS-E0003", ""),
        sample_with_anchor("RRS-E0004", "disk-full"),
    ];
    assert_eq!(
        duplicate_anchors(&error_anchors(&errors)),
        ["RRS-E0004 (catalog/errors/RRS-E.json): docs_anchor \"disk-full\" is used more than once"]
    );
    let terms = [
        sample_term("logit_lens", "logit-lens"),
        sample_term("probe", "logit-lens"),
    ];
    assert_eq!(
        duplicate_anchors(&term_anchors(&terms)),
        ["term probe (catalog/terms/terms.json): docs_anchor \"logit-lens\" is used more than once"]
    );
}

#[test]
fn a_term_listed_twice_is_caught() {
    let terms = [
        sample_term("logit_lens", "logit-lens"),
        sample_term("probe", "probe"),
        sample_term("logit_lens", "logit-lens-again"),
    ];
    assert_eq!(
        duplicate_term_ids(&terms),
        ["term logit_lens (catalog/terms/terms.json): listed more than once"]
    );
}

#[test]
fn error_files_are_recognised_by_path() {
    assert!(is_error_file("catalog/errors/RRS-E.json"));
    assert!(is_error_file("plugins/psychology/catalog/errors.json"));
    assert!(!is_error_file("catalog/errors/README.md"));
    assert!(!is_error_file("catalog/terms/terms.json"));
    assert!(!is_error_file("plugins/psychology/catalog/terms.json"));
    assert!(!is_error_file("plugins/psychology/rastai-plugin.json"));
}

// ---- catalog_ranges.rs equals contracts 10.1 ----

/// One prefix's row, read from the contracts 10.1 table or from catalog_ranges.rs, so the two compare directly.
#[derive(Debug, Clone)]
struct RangesRow {
    prefix: String,
    area: String,
    ranges: Vec<RangeEntry>,
}

/// One range of a row, both ends included.
#[derive(Debug, Clone)]
struct RangeEntry {
    first: u32,
    last: u32,
    area: String,
}

/// The rows of the contracts 10.1 table: the first run of table lines after the heading, less the header and divider.
/// Empty when the heading is missing.
fn contracts_ranges_rows(contracts_text: &str) -> Vec<RangesRow> {
    let Some((_, after_heading)) = contracts_text.split_once(RANGES_HEADING) else {
        return Vec::new();
    };
    let mut table_lines = Vec::new();
    for line in after_heading.lines() {
        if line.starts_with('|') {
            table_lines.push(line);
        } else if !table_lines.is_empty() {
            break; // the table ended
        }
    }
    let mut rows = Vec::new();
    for line in table_lines.iter().skip(TABLE_HEADER_LINES) {
        rows.push(parse_ranges_row(line));
    }
    rows
}

/// One table line: "| `RRS-E` | Formats and validation | `rastai-formats` | 0001–0999 syntax · ... |".
fn parse_ranges_row(line: &str) -> RangesRow {
    let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
    assert!(
        cells.len() > RANGES_COLUMN,
        "a contracts 10.1 row has too few cells: {line}"
    );
    let mut ranges = Vec::new();
    for range_text in cells[RANGES_COLUMN].split(RANGE_SEPARATOR) {
        ranges.push(parse_range(range_text));
    }
    RangesRow {
        prefix: cells[PREFIX_COLUMN].trim_matches('`').to_string(),
        area: cells[AREA_COLUMN].to_string(),
        ranges,
    }
}

/// One range: "0001–0999 syntax" is 1 to 999, about "syntax".
fn parse_range(range_text: &str) -> RangeEntry {
    let malformed = format!("a contracts 10.1 range is not \"0001–0999 area\": {range_text:?}");
    let (ends, area) = range_text.split_once(' ').expect(&malformed);
    let (first, last) = ends.split_once(RANGE_DASH).expect(&malformed);
    RangeEntry {
        first: first.parse().expect(&malformed),
        last: last.parse().expect(&malformed),
        area: area.to_string(),
    }
}

/// catalog_ranges.rs in the same shape as the contracts rows.
fn code_ranges_rows(prefixes: &[CatalogPrefix]) -> Vec<RangesRow> {
    let mut rows = Vec::new();
    for prefix_row in prefixes {
        let mut ranges = Vec::new();
        for code_range in prefix_row.ranges {
            ranges.push(RangeEntry {
                first: code_range.first,
                last: code_range.last,
                area: code_range.area.to_string(),
            });
        }
        rows.push(RangesRow {
            prefix: prefix_row.prefix.to_string(),
            area: prefix_row.area.to_string(),
            ranges,
        });
    }
    rows
}

/// Where the two tables differ: a prefix in only one of them, or a prefix whose rows differ.
fn ranges_table_differences(contracts_rows: &[RangesRow], code_rows: &[RangesRow]) -> Vec<String> {
    let mut problems = Vec::new();
    for contracts_row in contracts_rows {
        match code_rows.iter().find(|row| row.prefix == contracts_row.prefix) {
            Some(code_row) => problems.extend(row_differences(contracts_row, code_row)),
            None => problems.push(format!(
                "{}: in contracts 10.1, missing from catalog_ranges.rs",
                contracts_row.prefix
            )),
        }
    }
    for code_row in code_rows {
        if !contracts_rows.iter().any(|row| row.prefix == code_row.prefix) {
            problems.push(format!(
                "{}: in catalog_ranges.rs, missing from contracts 10.1",
                code_row.prefix
            ));
        }
    }
    problems
}

/// Where one prefix's two rows differ: its area, a range only one of them has, or a range's area.
fn row_differences(contracts_row: &RangesRow, code_row: &RangesRow) -> Vec<String> {
    let prefix = &contracts_row.prefix;
    let mut problems = Vec::new();
    if contracts_row.area != code_row.area {
        problems.push(format!(
            "{prefix}: area {:?} in contracts 10.1, {:?} in catalog_ranges.rs",
            contracts_row.area, code_row.area
        ));
    }
    for contracts_range in &contracts_row.ranges {
        let ends = format!("{prefix} {:04}–{:04}", contracts_range.first, contracts_range.last);
        match find_same_range(&code_row.ranges, contracts_range) {
            None => problems.push(format!("{ends}: in contracts 10.1, missing from catalog_ranges.rs")),
            Some(code_range) if !range_areas_match(&contracts_range.area, &code_range.area) => {
                problems.push(format!(
                    "{ends}: area {:?} in contracts 10.1, {:?} in catalog_ranges.rs",
                    contracts_range.area, code_range.area
                ));
            }
            Some(_) => {}
        }
    }
    for code_range in &code_row.ranges {
        if find_same_range(&contracts_row.ranges, code_range).is_none() {
            problems.push(format!(
                "{prefix} {:04}–{:04}: in catalog_ranges.rs, missing from contracts 10.1",
                code_range.first, code_range.last
            ));
        }
    }
    problems
}

/// The range with the same two ends, if the list has one.
fn find_same_range<'a>(ranges: &'a [RangeEntry], wanted: &RangeEntry) -> Option<&'a RangeEntry> {
    ranges
        .iter()
        .find(|candidate| candidate.first == wanted.first && candidate.last == wanted.last)
}

/// The areas match when they are equal, or when contracts adds only a note in brackets after the same words.
fn range_areas_match(contracts_area: &str, code_area: &str) -> bool {
    match contracts_area.strip_prefix(code_area) {
        Some(rest) => rest.is_empty() || rest.starts_with(AREA_NOTE_START),
        None => false,
    }
}

#[test]
fn catalog_ranges_equal_the_contracts_table() {
    let contracts_path = find_repo_root().unwrap().join(CONTRACTS_FILE);
    let contracts_text = std::fs::read_to_string(&contracts_path).expect("contracts.md reads");
    let contracts_rows = contracts_ranges_rows(&contracts_text);
    assert!(
        !contracts_rows.is_empty(),
        "no table after {RANGES_HEADING:?} in {CONTRACTS_FILE}"
    );
    let code_rows = code_ranges_rows(CATALOG_PREFIXES);
    assert_eq!(
        ranges_table_differences(&contracts_rows, &code_rows),
        Vec::<String>::new()
    );
}

#[test]
fn matching_sample_tables_have_no_differences() {
    let contracts_rows = contracts_ranges_rows(SAMPLE_CONTRACTS);
    assert_eq!(contracts_rows.len(), 2, "{contracts_rows:#?}");
    // RRS-U 1000–1999 also shows that a note in brackets after the same area words is no difference.
    assert_eq!(
        ranges_table_differences(&contracts_rows, &code_ranges_rows(SAMPLE_PREFIXES)),
        Vec::<String>::new()
    );
}

#[test]
fn a_range_in_only_one_table_is_caught() {
    let code_rows = code_ranges_rows(SAMPLE_PREFIXES);
    let wider_contracts = SAMPLE_CONTRACTS.replace("1000–1999 structure", "1000–1999 structure · 2000–2999 meaning");
    assert_eq!(
        ranges_table_differences(&contracts_ranges_rows(&wider_contracts), &code_rows),
        ["RRS-E 2000–2999: in contracts 10.1, missing from catalog_ranges.rs"]
    );
    let mut wider_code_rows = code_rows.clone();
    wider_code_rows[1].ranges.push(RangeEntry {
        first: 100,
        last: 199,
        area: "desktop shell".to_string(),
    });
    assert_eq!(
        ranges_table_differences(&contracts_ranges_rows(SAMPLE_CONTRACTS), &wider_code_rows),
        ["RRS-U 0100–0199: in catalog_ranges.rs, missing from contracts 10.1"]
    );
}

#[test]
fn a_moved_range_end_is_caught_on_both_sides() {
    let moved_contracts = SAMPLE_CONTRACTS.replace("0001–0099 connection", "0001–0199 connection");
    assert_eq!(
        ranges_table_differences(
            &contracts_ranges_rows(&moved_contracts),
            &code_ranges_rows(SAMPLE_PREFIXES)
        ),
        [
            "RRS-U 0001–0199: in contracts 10.1, missing from catalog_ranges.rs",
            "RRS-U 0001–0099: in catalog_ranges.rs, missing from contracts 10.1",
        ]
    );
}

#[test]
fn a_prefix_or_an_area_in_only_one_table_is_caught() {
    let code_rows = code_ranges_rows(SAMPLE_PREFIXES);
    let mut fewer_contracts = String::new();
    for line in SAMPLE_CONTRACTS.lines() {
        if !line.starts_with("| `RRS-U`") {
            fewer_contracts.push_str(line);
            fewer_contracts.push('\n');
        }
    }
    assert_eq!(
        ranges_table_differences(&contracts_ranges_rows(&fewer_contracts), &code_rows),
        ["RRS-U: in catalog_ranges.rs, missing from contracts 10.1"]
    );
    let renamed_contracts = SAMPLE_CONTRACTS
        .replace("| Formats and validation |", "| Formats |")
        .replace("1000–1999 structure", "1000–1999 layout");
    assert_eq!(
        ranges_table_differences(&contracts_ranges_rows(&renamed_contracts), &code_rows),
        [
            "RRS-E: area \"Formats\" in contracts 10.1, \"Formats and validation\" in catalog_ranges.rs",
            "RRS-E 1000–1999: area \"layout\" in contracts 10.1, \"structure\" in catalog_ranges.rs",
        ]
    );
    assert_eq!(
        ranges_table_differences(&[], &code_rows).len(),
        code_rows.len(),
        "a missing table reports every prefix"
    );
}

// ---- The catalog equals contracts 10.3 ----

/// One row of the contracts 10.3 tables: a code, its severity, and whether a milestone marker after the code puts it
/// in a later milestone's catalog ("`RRS-L0001` (M4)": the plugins' catalogs, written by M4-E01 and M5-E01).
#[derive(Debug, Clone)]
struct ContractsCode {
    code: String,
    severity: String,
    later_milestone: bool,
}

/// The rows of every table under the 10.3 heading, up to the next section. Empty when the heading is missing.
fn contracts_codes(contracts_text: &str) -> Vec<ContractsCode> {
    let Some((_, after_heading)) = contracts_text.split_once(CODES_HEADING) else {
        return Vec::new();
    };
    let section = match after_heading.split_once(NEXT_SECTION_START) {
        Some((section, _)) => section,
        None => after_heading,
    };
    let mut codes = Vec::new();
    for line in section.lines() {
        if line.starts_with(CODE_ROW_START) {
            codes.push(parse_code_row(line));
        }
    }
    codes
}

/// One table line: "| `RRS-L0001` (M4) | error | Replay weights not present |".
fn parse_code_row(line: &str) -> ContractsCode {
    let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
    assert!(
        cells.len() > SEVERITY_COLUMN,
        "a contracts 10.3 row has too few cells: {line}"
    );
    let code_cell = cells[CODE_COLUMN];
    let code = code_cell.split('`').nth(1).unwrap_or_default();
    // Whatever follows the closing backtick is a marker: "(M4)", "(M5)".
    let after_code = code_cell.rsplit('`').next().unwrap_or_default();
    ContractsCode {
        code: code.to_string(),
        severity: cells[SEVERITY_COLUMN].to_string(),
        later_milestone: !after_code.trim().is_empty(),
    }
}

/// Where the catalog and contracts 10.3 differ: a code in only one of them, or a code whose severity differs. A row
/// marked for a later milestone may be missing from the catalog, but once its entry exists it must match.
fn catalog_code_differences(contracts: &[ContractsCode], errors: &[ErrorRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for row in contracts {
        match errors.iter().find(|record| record.code == row.code) {
            None if row.later_milestone => {}
            None => problems.push(format!("{}: in contracts 10.3, missing from the catalog", row.code)),
            Some(record) if severity_word(record.severity) != row.severity => problems.push(format!(
                "{} ({}): severity {} in contracts 10.3, {} in the catalog",
                row.code,
                record.source_file,
                row.severity,
                severity_word(record.severity)
            )),
            Some(_) => {}
        }
    }
    for record in errors {
        if !contracts.iter().any(|row| row.code == record.code) {
            problems.push(format!(
                "{} ({}): in the catalog, missing from contracts 10.3 (list a new code there first)",
                record.code, record.source_file
            ));
        }
    }
    problems
}

#[test]
fn catalog_codes_equal_the_contracts_codes_table() {
    let contracts_path = find_repo_root().unwrap().join(CONTRACTS_FILE);
    let contracts_text = std::fs::read_to_string(&contracts_path).expect("contracts.md reads");
    let contracts = contracts_codes(&contracts_text);
    assert!(
        !contracts.is_empty(),
        "no rows after {CODES_HEADING:?} in {CONTRACTS_FILE}"
    );
    assert_eq!(
        catalog_code_differences(&contracts, &real_errors()),
        Vec::<String>::new()
    );
}

#[test]
fn contracts_code_rows_are_read_with_their_markers() {
    let codes = contracts_codes(SAMPLE_CODES_SECTION);
    let read: Vec<(&str, &str, bool)> = codes
        .iter()
        .map(|row| (row.code.as_str(), row.severity.as_str(), row.later_milestone))
        .collect();
    assert_eq!(
        read,
        [
            ("RRS-E0001", "error", false),
            ("RRS-E4001", "info", false),
            ("RRS-L0001", "error", true),
        ]
    );
    assert!(contracts_codes("## 10. Error Catalog\n").is_empty());
}

#[test]
fn a_code_in_only_one_place_or_with_another_severity_is_caught() {
    let contracts = contracts_codes(SAMPLE_CODES_SECTION);
    let mut newer_file = sample("RRS-E4001", CORE_E_FILE);
    newer_file.severity = Severity::Warning;
    let errors = [
        sample("RRS-E0001", CORE_E_FILE),
        newer_file,
        sample("RRS-E0002", CORE_E_FILE),
    ];
    assert_eq!(
        catalog_code_differences(&contracts, &errors),
        [
            "RRS-E4001 (catalog/errors/RRS-E.json): severity info in contracts 10.3, warning in the catalog",
            "RRS-E0002 (catalog/errors/RRS-E.json): in the catalog, missing from contracts 10.3 (list a new code there first)",
        ]
    );
    let mut matching_file = sample("RRS-E4001", CORE_E_FILE);
    matching_file.severity = Severity::Info;
    let without_the_first = [matching_file];
    assert_eq!(
        catalog_code_differences(&contracts, &without_the_first),
        ["RRS-E0001: in contracts 10.3, missing from the catalog"]
    );
}

#[test]
fn a_later_milestone_code_may_wait_but_must_match_once_written() {
    let contracts = contracts_codes(SAMPLE_CODES_SECTION);
    let mut info_file = sample("RRS-E4001", CORE_E_FILE);
    info_file.severity = Severity::Info;
    let mut wrong_plugin_entry = sample("RRS-L0001", "plugins/psychology/catalog/errors.json");
    wrong_plugin_entry.severity = Severity::Warning;
    let errors = [sample("RRS-E0001", CORE_E_FILE), info_file, wrong_plugin_entry];
    assert_eq!(catalog_code_differences(&contracts, &errors[..2]), Vec::<String>::new());
    assert_eq!(
        catalog_code_differences(&contracts, &errors),
        [
            "RRS-L0001 (plugins/psychology/catalog/errors.json): severity error in contracts 10.3, warning in the catalog"
        ]
    );
}

// ---- The source names only catalog codes ----

/// One code a source file names, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeMention {
    code: String,
    place: String,
}

/// Every well-formed code ("RRS-", a capital letter, four digits, then no further letter or digit) in a text, with
/// its file and line. Comments count: a comment that names a code points readers at it.
fn code_mentions(text: &str, file_name: &str) -> Vec<CodeMention> {
    let mut mentions = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(start) = rest.find(CODE_START) {
            let candidate = &rest[start..];
            let code = candidate.get(..CODE_LENGTH).unwrap_or(candidate);
            let next_character = candidate[code.len()..].chars().next();
            let ends_cleanly = !next_character.is_some_and(|character| character.is_ascii_alphanumeric());
            if split_code(code).is_some() && ends_cleanly {
                mentions.push(CodeMention {
                    code: code.to_string(),
                    place: format!("{file_name}:{}", line_index + 1),
                });
            }
            rest = &candidate[CODE_START.len()..];
        }
    }
    mentions
}

/// The part of a file the scans read. Tests name made-up codes on purpose (an unknown code, one outside every range),
/// so a Rust file is read up to its unit-test module, and test files are skipped whole (`is_scanned_file`).
/// Production code after the test module would be missed, but clippy's `items_after_test_module` (a default lint,
/// an error under `just lint`'s `-D warnings`) already refuses any.
fn scanned_text(text: &str, file_name: &str) -> String {
    if !file_name.ends_with(RUST_EXTENSION) {
        return text.to_string();
    }
    match unit_test_module_start(text) {
        Some(test_start) => text[..test_start].to_string(),
        None => text.to_string(),
    }
}

/// Where the first `#[cfg(test)]` that opens an inline module stands. One on a helper function (production code may
/// follow it) or on `mod name;` (a module in its own file) is skipped.
fn unit_test_module_start(text: &str) -> Option<usize> {
    for (index, _) in text.match_indices(RUST_TEST_ATTRIBUTE) {
        let item = item_after_attribute(&text[index + RUST_TEST_ATTRIBUTE.len()..]);
        if is_inline_module(item) {
            return Some(index);
        }
    }
    None
}

/// The text from the item an attribute belongs to: past blank space, further attributes (`#[allow(...)]`) and
/// comment lines.
fn item_after_attribute(text: &str) -> &str {
    let mut rest = text.trim_start();
    while rest.starts_with(RUST_ATTRIBUTE_START) || rest.starts_with(RUST_COMMENT_START) {
        rest = match rest.split_once('\n') {
            Some((_, next_lines)) => next_lines.trim_start(),
            None => "",
        };
    }
    rest
}

/// True for `mod name {`, a module written in place; `mod name;` ends at its semicolon, before any brace.
fn is_inline_module(item: &str) -> bool {
    let Some(after_mod) = item.strip_prefix(RUST_MODULE_START) else {
        return false;
    };
    let header_end = after_mod.find(['{', ';']);
    header_end.is_some_and(|end| after_mod[end..].starts_with('{'))
}

/// Source files outside test folders and build output, by extension. Test files (`test_*.py`, `conftest.py`,
/// `*.test.js`, `*.test.jsx`) are skipped like test folders.
fn is_scanned_file(relative_path: &str) -> bool {
    let parts: Vec<&str> = relative_path.split('/').collect();
    let Some((file_name, folders)) = parts.split_last() else {
        return false;
    };
    if folders.iter().any(|folder| SKIPPED_FOLDERS.contains(folder)) {
        return false;
    }
    let is_test_file = file_name.starts_with(PYTHON_TEST_FILE_START)
        || *file_name == PYTHON_TEST_SETUP_FILE
        || JAVASCRIPT_TEST_FILE_ENDS.iter().any(|end| file_name.ends_with(end));
    let has_scanned_extension = SCANNED_EXTENSIONS
        .iter()
        .any(|extension| file_name.ends_with(extension));
    has_scanned_extension && !is_test_file
}

/// Every scanned file under the scanned folders, relative to the repository root with `/` separators, sorted.
fn scanned_files(repo_root: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    for folder in SCANNED_FOLDERS {
        collect_files(repo_root, folder, &mut files);
    }
    files.retain(|file| is_scanned_file(file));
    files.sort();
    files
}

/// Adds every file under `folder` (relative to the root) to `files`. Skipped folders aren't entered, so build output
/// and installed packages cost nothing. A folder that doesn't exist yet (apps/, plugins/) adds nothing.
fn collect_files(repo_root: &std::path::Path, folder: &str, files: &mut Vec<String>) {
    let Ok(listing) = std::fs::read_dir(repo_root.join(folder)) else {
        return;
    };
    for item in listing {
        let item = item.expect("a folder lists");
        let name = item.file_name().into_string().expect("file names are UTF-8");
        let relative = format!("{folder}/{name}");
        let file_type = item.file_type().expect("a file type reads");
        if file_type.is_dir() && !SKIPPED_FOLDERS.contains(&name.as_str()) {
            collect_files(repo_root, &relative, files);
        } else if file_type.is_file() {
            files.push(relative);
        }
    }
}

/// Mentions of codes the catalog doesn't hold.
fn mentions_missing_from_catalog(mentions: &[CodeMention], errors: &[ErrorRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for mention in mentions {
        if !errors.iter().any(|record| record.code == mention.code) {
            problems.push(format!("{}: {} is not in the catalog", mention.place, mention.code));
        }
    }
    problems
}

#[test]
fn no_source_names_a_code_missing_from_the_catalog() {
    let repo_root = find_repo_root().unwrap();
    let mut mentions = Vec::new();
    for file in scanned_files(&repo_root) {
        let Ok(text) = std::fs::read_to_string(repo_root.join(&file)) else {
            continue; // not text, so it names no code
        };
        mentions.extend(code_mentions(&scanned_text(&text, &file), &file));
    }
    assert!(
        !mentions.is_empty(),
        "the scan found no code at all; is it reading the source?"
    );
    assert_eq!(
        mentions_missing_from_catalog(&mentions, &real_errors()),
        Vec::<String>::new()
    );
}

#[test]
fn code_mentions_are_found_with_their_lines() {
    let text = "a \"RRS-E0001\" b\n// RRS-S3001, `RRS-D4xxx`, RRS-E10030 and RRS-e0001\nRRS-C0401.";
    assert_eq!(
        code_mentions(text, "src/a.rs"),
        [
            CodeMention {
                code: "RRS-E0001".to_string(),
                place: "src/a.rs:1".to_string()
            },
            CodeMention {
                code: "RRS-S3001".to_string(),
                place: "src/a.rs:2".to_string()
            },
            CodeMention {
                code: "RRS-C0401".to_string(),
                place: "src/a.rs:3".to_string()
            },
        ]
    );
    let errors = [sample("RRS-E0001", CORE_E_FILE)];
    assert_eq!(
        mentions_missing_from_catalog(&code_mentions(text, "src/a.rs"), &errors),
        [
            "src/a.rs:2: RRS-S3001 is not in the catalog",
            "src/a.rs:3: RRS-C0401 is not in the catalog"
        ]
    );
}

#[test]
fn tests_and_build_output_are_not_scanned() {
    assert!(is_scanned_file("crates/rastai-formats/src/error.rs"));
    assert!(is_scanned_file("python/rastai_data/rastai_data/errors.py"));
    assert!(is_scanned_file("packages/client/src/client.js"));
    assert!(is_scanned_file("proto/rastai/events/v1/event_service.proto"));
    assert!(!is_scanned_file("crates/rastai-formats/tests/documents_corpus.rs"));
    assert!(!is_scanned_file("python/rastai_data/tests/test_readers.py"));
    assert!(!is_scanned_file("python/conftest.py"));
    assert!(!is_scanned_file("packages/proto/src/golden.test.js"));
    assert!(!is_scanned_file("tools/codegen/target/debug/build.rs"));
    assert!(!is_scanned_file("packages/catalog/node_modules/x/index.js"));
    assert!(!is_scanned_file("crates/rastai-formats/src/notes.md"));
    let rust = "const CODE: &str = \"RRS-E0001\";\n#[cfg(test)]\nmod tests { const MISSING: &str = \"RRS-E9999\"; }\n";
    assert_eq!(scanned_text(rust, "src/a.rs"), "const CODE: &str = \"RRS-E0001\";\n");
    assert_eq!(scanned_text("RRS-E9999", "a/b.py"), "RRS-E9999");
}

#[test]
fn a_test_helper_or_a_test_module_file_does_not_hide_the_code_after_it() {
    let rust = r#"#[cfg(test)]
mod test_random;

#[cfg(test)]
pub(crate) fn count(map: &Map) -> usize { 0 }

const CODE: &str = "RRS-E0998";

#[cfg(test)]
#[allow(clippy::unwrap_used)]
// The unit tests.
mod tests {
    const MADE_UP: &str = "RRS-E9999";
}
"#;
    let scanned = scanned_text(rust, "src/a.rs");
    assert!(scanned.contains("RRS-E0998"), "{scanned}");
    assert!(!scanned.contains("RRS-E9999"), "{scanned}");
    assert_eq!(
        mentions_missing_from_catalog(&code_mentions(&scanned, "src/a.rs"), &[]),
        ["src/a.rs:7: RRS-E0998 is not in the catalog"]
    );
}

// ---- Placeholders at Rust call sites ----

/// One `diagnostic(...)` or `diagnostic_at(...)` call whose code and values are written out at the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallSite {
    code: String,
    value_names: Vec<String>,
    place: String,
}

/// The call sites a plain text search can read whole (layout rust-control-plane.md 2.6, no macro): the code is a
/// string literal or a `const NAME: &str = "RRS-...";` of the same file, and the values are an inline
/// `&[("name", ...), ...]` or `&values` where the closest `let values = ...` above the call is such an array. A call
/// that builds its values another way (`error_values(...)`, `borrow_values(...)`) is skipped; its own tests check its
/// message.
fn call_sites(text: &str, file_name: &str) -> Vec<CallSite> {
    // Each site with its position, so the list comes out in file order whichever function found it.
    let mut sites_by_position = Vec::new();
    for function_name in DIAGNOSTIC_FUNCTIONS {
        let call_start = format!("{function_name}(");
        for (index, _) in text.match_indices(&call_start) {
            if is_part_of_longer_name(text, index) {
                continue; // syntax_diagnostic( is another function; `fn diagnostic(` passes, its argument is no code
            }
            let arguments = &text[index + call_start.len()..];
            if let Some(site) = read_call_site(text, arguments, file_name, index) {
                sites_by_position.push((index, site));
            }
        }
    }
    sites_by_position.sort_by_key(|(index, _)| *index);
    sites_by_position.into_iter().map(|(_, site)| site).collect()
}

/// True when the character before `index` continues a name, so `index` isn't the start of the function name.
fn is_part_of_longer_name(text: &str, index: usize) -> bool {
    text[..index]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// The code and value names of one call, from the text right after its opening bracket.
fn read_call_site(text: &str, arguments: &str, file_name: &str, call_index: usize) -> Option<CallSite> {
    let (code_argument, after_code) = arguments.split_once(',')?;
    let code = resolve_code(code_argument.trim(), text)?;
    let values = values_array(after_code.trim_start(), &text[..call_index])?;
    let line = text[..call_index].matches('\n').count() + 1;
    Some(CallSite {
        code,
        value_names: tuple_names(values),
        place: format!("{file_name}:{line}"),
    })
}

/// The array text after its `[`, for a values argument that is `&[...]` inline or `&name` declared above the call.
fn values_array<'a>(values_argument: &'a str, text_before_call: &'a str) -> Option<&'a str> {
    if let Some(inline) = values_argument.strip_prefix(VALUES_ARRAY_START) {
        return Some(inline);
    }
    let variable = values_variable(values_argument)?;
    declared_array(text_before_call, variable)
}

/// `values` for an argument `&values` that ends there (`&values,` or `&values)`); anything longer, such as
/// `&borrow_values(&values)` or `&values[1..]`, is no plain variable.
fn values_variable(values_argument: &str) -> Option<&str> {
    let after_reference = values_argument.strip_prefix(VALUES_REFERENCE_START)?;
    let name_length = after_reference
        .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .unwrap_or(after_reference.len());
    let next_character = after_reference[name_length..].trim_start().chars().next();
    let ends_there = next_character == Some(',') || next_character == Some(')');
    (name_length > 0 && ends_there).then(|| &after_reference[..name_length])
}

/// The array text after `let name = [` for the closest declaration of `name` above the call. When that closest
/// declaration is not an array (`let values = error_values(error);`), there is nothing to read: an older array of the
/// same name further up isn't the one the call passes.
fn declared_array<'a>(text_before_call: &'a str, variable: &str) -> Option<&'a str> {
    let declaration = format!("let {variable} = ");
    let declaration_start = text_before_call.rfind(&declaration)?;
    let after_declaration = &text_before_call[declaration_start + declaration.len()..];
    after_declaration.strip_prefix(ARRAY_START)
}

/// A code argument: a string literal, or the name of a `const` in the same file whose value is one.
fn resolve_code(argument: &str, text: &str) -> Option<String> {
    if let Some(literal) = argument.strip_prefix('"') {
        return Some(literal.trim_end_matches('"').to_string());
    }
    let declaration = format!("const {argument}: &str = \"");
    let (_, after_declaration) = text.split_once(&declaration)?;
    let (code, _) = after_declaration.split_once('"')?;
    Some(code.to_string())
}

/// The first string of each tuple in an array's text, up to the array's closing bracket: `("name", value), ...]`
/// gives ["name"]. Depth counting keeps brackets inside the values (`&format!("{}", x)`) out of the way, and strings
/// are skipped whole so a bracket inside one doesn't count.
fn tuple_names(array_text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 1; // inside the array's [
    let mut characters = array_text.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => {
                let string_end = string_length(&array_text[index..]);
                if depth == 2 && is_first_in_tuple(&array_text[..index]) {
                    names.push(array_text[index + 1..index + string_end - 1].to_string());
                }
                // Skip the rest of the string.
                for _ in 1..array_text[index..index + string_end].chars().count() {
                    characters.next();
                }
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    break; // the array ended
                }
            }
            _ => {}
        }
    }
    names
}

/// The length in bytes of the string literal at the start of `text`, quotes included; an escaped quote doesn't end it.
fn string_length(text: &str) -> usize {
    let mut escaped = false;
    for (index, character) in text.char_indices().skip(1) {
        match character {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return index + 1,
            _ => escaped = false,
        }
    }
    text.len()
}

/// True when only spaces stand between a tuple's opening bracket and this point: the string is the tuple's first item.
fn is_first_in_tuple(text_before: &str) -> bool {
    text_before.trim_end().ends_with('(')
}

/// The `{name}` placeholders of a catalog text.
fn placeholder_names(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (open_index, _) in template.match_indices('{') {
        let after_open = &template[open_index + 1..];
        let Some(close_index) = after_open.find('}') else {
            continue;
        };
        let name = &after_open[..close_index];
        let is_name = name.starts_with(|first: char| first.is_ascii_lowercase())
            && name
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_');
        if is_name {
            names.push(name.to_string());
        }
    }
    names
}

/// Where a call site and its entry disagree: a value the message and fix never use, or a placeholder the call site
/// doesn't fill (it would show as `{name}`). A code missing from the catalog is the source scan's problem.
fn placeholder_problems(sites: &[CallSite], errors: &[ErrorRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for site in sites {
        let Some(record) = errors.iter().find(|record| record.code == site.code) else {
            continue;
        };
        // A name may sit in both the message and the fix ("free {needed_bytes} bytes"); it counts once.
        let mut used = BTreeSet::new();
        used.extend(placeholder_names(&record.message));
        used.extend(placeholder_names(&record.fix_text));
        for name in &site.value_names {
            if !used.contains(name) {
                problems.push(format!(
                    "{}: passes {name} to {}, whose message and fix don't use it",
                    site.place, site.code
                ));
            }
        }
        for name in &used {
            if !site.value_names.contains(name) {
                problems.push(format!(
                    "{}: {} uses {{{name}}}, which the call doesn't pass",
                    site.place, site.code
                ));
            }
        }
    }
    problems
}

#[test]
fn call_site_placeholders_match_their_entries() {
    let repo_root = find_repo_root().unwrap();
    let mut sites = Vec::new();
    for file in scanned_files(&repo_root) {
        if !file.ends_with(RUST_EXTENSION) {
            continue;
        }
        let text = std::fs::read_to_string(repo_root.join(&file)).expect("a Rust file reads");
        sites.extend(call_sites(&scanned_text(&text, &file), &file));
    }
    assert_eq!(placeholder_problems(&sites, &real_errors()), Vec::<String>::new());
}

#[test]
fn call_sites_are_read_from_literals_and_same_file_constants() {
    let text = r#"const CODE_DISK_FULL: &str = "RRS-S3001";
fn check() {
    let a = diagnostic("RRS-E2004", &[("input_name", name), ("other", &format!("({})", x))]);
    let b = diagnostic_at(CODE_DISK_FULL, &[("operation", "Saving \"x\" ]")], path);
    let c = diagnostic_at(code, &[("skipped", x)], path);
    let d = diagnostic("RRS-E1003", &values);
    let e = syntax_diagnostic("RRS-E0001", &[("skipped", x)]);
}
fn declared_above() {
    let values = [("old", x)];
    let values = [
        ("found", found.as_str()),
        ("expected", EXPECTED),
    ];
    let f = diagnostic_at("RRS-E1004", &values, &path);
    let g = version_diagnostic("RRS-E1004", &values, input);
    let h = diagnostic("RRS-E1004", &borrow_values(&values));
}
fn declared_another_way() {
    let values = error_values(error);
    let i = diagnostic("RRS-E1004", &values);
}
"#;
    let sites = call_sites(text, "src/a.rs");
    let site = |code: &str, names: &[&str], line: usize| CallSite {
        code: code.to_string(),
        value_names: names.iter().map(|name| name.to_string()).collect(),
        place: format!("src/a.rs:{line}"),
    };
    assert_eq!(
        sites,
        [
            site("RRS-E2004", &["input_name", "other"], 3),
            site("RRS-S3001", &["operation"], 4),
            site("RRS-E1004", &["found", "expected"], 15),
            site("RRS-E1004", &["found", "expected"], 16),
        ]
    );
}

#[test]
fn unused_values_and_unfilled_placeholders_are_caught() {
    let mut record = sample("RRS-S3001", "catalog/errors/RRS-S.json");
    record.message = "{operation} needs {needed_bytes} bytes.".to_string();
    record.fix_text = "Free {needed_bytes} bytes.".to_string();
    let site = |names: &[&str]| CallSite {
        code: "RRS-S3001".to_string(),
        value_names: names.iter().map(|name| name.to_string()).collect(),
        place: "src/a.rs:7".to_string(),
    };
    let errors = [record];
    assert_eq!(
        placeholder_problems(&[site(&["operation", "needed_bytes"])], &errors),
        Vec::<String>::new()
    );
    assert_eq!(
        placeholder_problems(&[site(&["operation", "volume"])], &errors),
        [
            "src/a.rs:7: passes volume to RRS-S3001, whose message and fix don't use it",
            "src/a.rs:7: RRS-S3001 uses {needed_bytes}, which the call doesn't pass",
        ]
    );
    assert_eq!(placeholder_names("{a} {b_2} {Not} {} { c} {"), ["a", "b_2"]);
}
