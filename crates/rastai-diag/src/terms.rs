//! Term lookup: the plain-English definitions the UI shows for words like "logit lens"
//! (the glossary, contracts section 13; layout: rust-control-plane.md section 2.6).
//!
//! The entries live in `generated/terms_table.rs`, written by `rastai-catalog-codegen` from `catalog/terms/terms.json`.

use crate::generated::terms_table::TERMS;

/// One term, as `rastai-catalog-codegen` writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermEntry {
    pub term_id: &'static str,
    pub title: &'static str,
    pub basic_text: &'static str,
    pub advanced_text: &'static str,
    pub docs_anchor: &'static str,
}

/// The term with this id, or None. A linear scan, like `find_entry`: the table is small.
pub fn find_term(term_id: &str) -> Option<&'static TermEntry> {
    TERMS.iter().find(|term| term.term_id == term_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_term_has_no_entry() {
        assert_eq!(find_term("no_such_term"), None);
    }
}
