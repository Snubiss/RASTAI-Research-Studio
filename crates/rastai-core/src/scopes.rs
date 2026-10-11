//! Permission scopes (contracts 6.3): what a principal may do, and the text form commands, tokens and manifests use.
//!
//! The 17 fixed scopes are written as contracts 6.3 lists them (`conversations.read`). A third-party plugin's own
//! scope is written `plugin:<plugin_id>`: the colon can't appear in a fixed scope or a plugin id
//! (`[a-z][a-z0-9_]*`, contracts 6.1), so the two kinds can never be read as each other.

use rastai_diag::diagnostic::{Diagnostic, diagnostic};

const PLUGIN_SCOPE_PREFIX: &str = "plugin:";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";

// Scoped-token presets (contracts 6.3).
const CHAT_ONLY_PRESET: &str = "chat-only";
const MCP_DEFAULT_PRESET: &str = "mcp-default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Read,
    Chat,
    ConversationsRead,
    GatewaySpend,
    InferenceManage,
    Psychology,
    Training,
    Eval,
    Designer,
    Gym,
    SandboxSpawn,
    Data,
    Project,
    PluginsManage,
    Remote,
    Secrets,
    SystemAdmin,
    /// A third-party plugin's own scope, named by its plugin id.
    Plugin(String),
}

/// Every fixed scope, in contracts 6.3 order. The owner holds all of them.
pub const FIXED_SCOPES: &[Scope] = &[
    Scope::Read,
    Scope::Chat,
    Scope::ConversationsRead,
    Scope::GatewaySpend,
    Scope::InferenceManage,
    Scope::Psychology,
    Scope::Training,
    Scope::Eval,
    Scope::Designer,
    Scope::Gym,
    Scope::SandboxSpawn,
    Scope::Data,
    Scope::Project,
    Scope::PluginsManage,
    Scope::Remote,
    Scope::Secrets,
    Scope::SystemAdmin,
];

/// The scope's text: `conversations.read`, or `plugin:<plugin_id>`.
pub fn scope_name(scope: &Scope) -> String {
    match scope {
        Scope::Read => "read".to_string(),
        Scope::Chat => "chat".to_string(),
        Scope::ConversationsRead => "conversations.read".to_string(),
        Scope::GatewaySpend => "gateway.spend".to_string(),
        Scope::InferenceManage => "inference.manage".to_string(),
        Scope::Psychology => "psychology".to_string(),
        Scope::Training => "training".to_string(),
        Scope::Eval => "eval".to_string(),
        Scope::Designer => "designer".to_string(),
        Scope::Gym => "gym".to_string(),
        Scope::SandboxSpawn => "sandbox.spawn".to_string(),
        Scope::Data => "data".to_string(),
        Scope::Project => "project".to_string(),
        Scope::PluginsManage => "plugins.manage".to_string(),
        Scope::Remote => "remote".to_string(),
        Scope::Secrets => "secrets".to_string(),
        Scope::SystemAdmin => "system.admin".to_string(),
        Scope::Plugin(plugin_id) => format!("{PLUGIN_SCOPE_PREFIX}{plugin_id}"),
    }
}

/// The scope a text names. Anything else is `RRS-E1004` (not one of the allowed values).
// The layout's shape for every check that refuses with one diagnostic (5.2). The refusal is the rare path, so its
// size costs nothing in practice, and boxing it would add noise at every call site.
#[allow(clippy::result_large_err)]
pub fn parse_scope(text: &str) -> Result<Scope, Diagnostic> {
    if let Some(plugin_id) = text.strip_prefix(PLUGIN_SCOPE_PREFIX) {
        if is_plugin_id(plugin_id) {
            return Ok(Scope::Plugin(plugin_id.to_string()));
        }
        return Err(unknown_scope(text));
    }
    match FIXED_SCOPES.iter().find(|scope| scope_name(scope) == text) {
        Some(scope) => Ok(scope.clone()),
        None => Err(unknown_scope(text)),
    }
}

/// The scopes of a scoped-token preset (`chat-only`, `mcp-default`), or None for a name that isn't one.
pub fn token_preset_scopes(preset: &str) -> Option<Vec<Scope>> {
    match preset {
        CHAT_ONLY_PRESET => Some(vec![Scope::Chat]),
        MCP_DEFAULT_PRESET => Some(vec![Scope::Read, Scope::Chat, Scope::Project]),
        _ => None,
    }
}

/// True when `needed` is among the `held` scopes. No scope implies another: `system.admin` doesn't grant `chat`,
/// so a token can be given exactly what it needs.
pub fn scopes_allow(held: &[Scope], needed: &Scope) -> bool {
    held.contains(needed)
}

/// A plugin id as contracts 6.1 defines it: `[a-z][a-z0-9_]*`.
fn is_plugin_id(text: &str) -> bool {
    let mut characters = text.bytes();
    match characters.next() {
        Some(first) if first.is_ascii_lowercase() => {
            characters.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }
        _ => false,
    }
}

fn unknown_scope(text: &str) -> Diagnostic {
    let names: Vec<String> = FIXED_SCOPES.iter().map(scope_name).collect();
    let expected = format!("one of: {}, or {PLUGIN_SCOPE_PREFIX}<plugin_id>", names.join(", "));
    diagnostic(CODE_VALUE_NOT_ALLOWED, &[("found", text), ("expected", &expected)])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // Contracts 6.3, in its order.
    const CONTRACT_SCOPE_NAMES: &[&str] = &[
        "read",
        "chat",
        "conversations.read",
        "gateway.spend",
        "inference.manage",
        "psychology",
        "training",
        "eval",
        "designer",
        "gym",
        "sandbox.spawn",
        "data",
        "project",
        "plugins.manage",
        "remote",
        "secrets",
        "system.admin",
    ];

    #[test]
    fn fixed_scopes_have_the_contract_names() {
        let names: Vec<String> = FIXED_SCOPES.iter().map(scope_name).collect();
        assert_eq!(names, CONTRACT_SCOPE_NAMES);
    }

    #[test]
    fn every_scope_round_trips_through_its_name() {
        let mut scopes = FIXED_SCOPES.to_vec();
        scopes.push(Scope::Plugin("acme_tools2".to_string()));
        for scope in scopes {
            assert_eq!(parse_scope(&scope_name(&scope)).unwrap(), scope);
        }
        assert_eq!(scope_name(&Scope::Plugin("acme".to_string())), "plugin:acme");
    }

    #[test]
    fn unknown_names_are_refused() {
        for text in [
            "",
            "Read",
            "admin",
            "conversations",
            "plugin:",
            "plugin:Acme",
            "plugin:1x",
            "plugin:a.b",
            "x:y",
        ] {
            let refusal = parse_scope(text).unwrap_err();
            assert_eq!(refusal.code, CODE_VALUE_NOT_ALLOWED, "{text:?}");
        }
    }

    #[test]
    fn presets_are_the_contract_ones() {
        assert_eq!(token_preset_scopes("chat-only").unwrap(), vec![Scope::Chat]);
        assert_eq!(
            token_preset_scopes("mcp-default").unwrap(),
            vec![Scope::Read, Scope::Chat, Scope::Project]
        );
        assert_eq!(token_preset_scopes("everything"), None);
    }

    #[test]
    fn a_scope_is_allowed_only_when_held() {
        let held = token_preset_scopes("mcp-default").unwrap();
        assert!(scopes_allow(&held, &Scope::Read));
        assert!(!scopes_allow(&held, &Scope::SystemAdmin));
        assert!(!scopes_allow(&[Scope::SystemAdmin], &Scope::Chat));
        let plugin_held = [Scope::Plugin("acme".to_string())];
        assert!(scopes_allow(&plugin_held, &Scope::Plugin("acme".to_string())));
        assert!(!scopes_allow(&plugin_held, &Scope::Plugin("other".to_string())));
        assert!(!scopes_allow(&[], &Scope::Read));
    }
}
