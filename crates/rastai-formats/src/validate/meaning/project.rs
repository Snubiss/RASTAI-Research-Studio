//! Layer 3 for `rastai.project/1`: the project has a name, and its default node is `local` or a node id
//! (`RRS-E2006` at the field).
//!
//! The data policy isn't checked here: it moved out of `project.json` into the project's `settings.json`
//! (`privacy.data_policy`, contracts 11.1), whose layer 2 already refuses an unknown policy. The project's folders are
//! not checked either: `read_document` sees one file, and a missing folder never stops a project from opening
//! (flows/platform.md, projects); `project.open` creates it.

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::project::Project;
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_VALUE_FORM: &str = "RRS-E2006";
const NAME_POINTER: &str = "/name";
const DEFAULT_NODE_POINTER: &str = "/default_node";
const EXPECTED_NAME: &str = "a project name: it needs at least one letter, digit or other visible character";
const EXPECTED_NODE: &str = "local, or a node id such as node_<base32 of its public key>";
/// The embedded node's name (contracts 11.2 `compute.default_node`).
const LOCAL_NODE: &str = "local";
/// Node ids are `node_` and the base32 of the node's public key (contracts 8.6).
const NODE_ID_PREFIX: &str = "node_";

/// Every meaning problem of a project document.
pub fn check_project(project: &Project, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if project.name.trim().is_empty() {
        let values = [("found", shown_value(&project.name)), ("expected", EXPECTED_NAME)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, NAME_POINTER));
    }
    // Empty means the user's compute.default_node setting.
    let node = project.default_node.as_str();
    if !node.is_empty() && !is_node_name(node) {
        let values = [("found", node), ("expected", EXPECTED_NODE)];
        diagnostics.push(meaning_diagnostic(
            CODE_VALUE_FORM,
            &values,
            source,
            DEFAULT_NODE_POINTER,
        ));
    }
    diagnostics
}

/// `local`, or `node_` followed by letters and digits. The exact alphabet and length of the encoded key belong to
/// rastai-core, which makes node ids (`node_id_from_public_key`); this catches a host name or a typo.
fn is_node_name(text: &str) -> bool {
    if text == LOCAL_NODE {
        return true;
    }
    let Some(encoded) = text.strip_prefix(NODE_ID_PREFIX) else {
        return false;
    };
    !encoded.is_empty() && encoded.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_node_ids_are_node_names() {
        assert!(is_node_name("local"));
        assert!(is_node_name("node_7Q2M9XKZ0ABCDEFGHJKMNPQRSTVWXYZ0123456789ABCDEFGHJK"));
    }

    #[test]
    fn other_text_is_not() {
        for text in [
            "",
            "Local",
            "node_",
            "node_7Q2M-X",
            "gpu-box",
            "node-7Q2M",
            "node_7Q2M ",
        ] {
            assert!(!is_node_name(text), "{text}");
        }
    }
}
