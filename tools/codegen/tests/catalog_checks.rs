//! The catalog rules (contracts 10; layout rust-control-plane.md 2.6 and 3.5): every code is well formed, unique,
//! inside a range its owner holds and in its owner's file, and never deleted once it reached the base branch (a code
//! that is no longer raised is marked `retired` instead, so its number is never reused). Every docs anchor is a
//! lowercase kebab-case slug, used once among the errors and once among the terms, and every term id is unique.
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

use rastai_codegen::catalog_ranges::{find_prefix, find_range, split_code};
use rastai_codegen::catalog_source::{ErrorRecord, TermRecord, read_catalog, read_error_codes};
use rastai_codegen::repo_root::find_repo_root;
use rastai_diag::diagnostic::Severity;

/// The branch a pull request merges into; CI passes the PR's base, as for check-schema-diff (contracts 12.6).
const BASE_REF_VARIABLE: &str = "RASTAI_BASE_REF";
const DEFAULT_BASE_REF: &str = "origin/main";

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

const CORE_E_FILE: &str = "catalog/errors/RRS-E.json";

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
        sample("RRS-C0500", "catalog/errors/RRS-C.json"),
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
            "RRS-C0500 (catalog/errors/RRS-C.json): 0500 is outside every RRS-C range (contracts 10.1)",
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
