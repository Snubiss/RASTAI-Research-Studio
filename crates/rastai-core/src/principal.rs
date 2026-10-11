//! Principals (contracts 6.3): who runs a command. The daemon derives the principal from the cookie, token, socket
//! peer or plugin job token, never from the request, and records `principal_label` on every journal entry and
//! `command.finished` event.

use crate::scopes::{FIXED_SCOPES, Scope};

const OWNER_LABEL: &str = "owner";
const LABEL_SEPARATOR: char = ':';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// The machine's user: the UI's cookie, a local socket peer or `owner.token`.
    Owner,
    /// A scoped token (`tok_...`), for MCP clients and scripts.
    Token,
    /// A plugin's module or job, or a plugin UI narrowing the owner (`acting_plugin_id`).
    Plugin,
    /// A paired node (`node_...`).
    Node,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub kind: PrincipalKind,
    /// The token id, plugin id or node id; empty for the owner, who is one per machine.
    pub id: String,
    pub scopes: Vec<Scope>,
}

/// The owner, with every fixed scope. A plugin's own scope (`Scope::Plugin`) can't be listed ahead of time, so
/// `scopes_allow` says no to the owner for one: `check_scope` (M1-21) lets the owner kind through for those
/// (contracts 6.3: the owner has every scope).
pub fn owner_principal() -> Principal {
    Principal {
        kind: PrincipalKind::Owner,
        id: String::new(),
        scopes: FIXED_SCOPES.to_vec(),
    }
}

/// `owner`, or `<kind>:<id>`: `token:tok_...`, `plugin:eval`, `node:node_...` (contracts 4.3, 6.3).
pub fn principal_label(principal: &Principal) -> String {
    match principal.kind {
        PrincipalKind::Owner => OWNER_LABEL.to_string(),
        PrincipalKind::Token => format!("token{LABEL_SEPARATOR}{}", principal.id),
        PrincipalKind::Plugin => format!("plugin{LABEL_SEPARATOR}{}", principal.id),
        PrincipalKind::Node => format!("node{LABEL_SEPARATOR}{}", principal.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scopes::scopes_allow;

    fn principal(kind: PrincipalKind, id: &str) -> Principal {
        Principal {
            kind,
            id: id.to_string(),
            scopes: Vec::new(),
        }
    }

    #[test]
    fn owner_holds_every_fixed_scope() {
        let owner = owner_principal();
        assert_eq!(owner.kind, PrincipalKind::Owner);
        for scope in FIXED_SCOPES {
            assert!(scopes_allow(&owner.scopes, scope), "{scope:?}");
        }
    }

    #[test]
    fn labels_are_kind_and_id() {
        assert_eq!(principal_label(&owner_principal()), "owner");
        assert_eq!(
            principal_label(&principal(PrincipalKind::Token, "tok_01J")),
            "token:tok_01J"
        );
        assert_eq!(
            principal_label(&principal(PrincipalKind::Plugin, "eval")),
            "plugin:eval"
        );
        assert_eq!(
            principal_label(&principal(PrincipalKind::Node, "node_7Z")),
            "node:node_7Z"
        );
    }
}
