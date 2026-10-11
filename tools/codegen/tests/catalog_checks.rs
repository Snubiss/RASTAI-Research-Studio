//! The catalog rules (contracts 10; layout rust-control-plane.md 2.6 and 3.5): every code is well formed, unique,
//! inside a range its owner holds and in its owner's file, and never deleted once it reached the base branch (a code
//! that is no longer raised is marked `retired` instead, so its number is never reused). Every docs anchor is a
//! lowercase kebab-case slug, used once among the errors and once among the terms, and every term id is unique.
//!
//! The owners' ranges in src/catalog_ranges.rs are a hand copy of the contracts 10.1 table, so one test reads that
//! table and compares the two: a prefix, an area or a range that only one of them has fails it, and neither can
//! drift from the other unnoticed.
//!
//! Two rules of layout 2.6 join later, with their inputs: placeholders used at Rust call sites (M1-13, with the
//! source scan) and `fix_command_id` naming a real command (M1-22, with the first rows of the command table).
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
