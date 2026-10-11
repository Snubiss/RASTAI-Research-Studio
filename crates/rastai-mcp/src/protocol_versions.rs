//! The MCP revisions this server speaks, and the choice made at `initialize` (layout: rust-control-plane.md 7.1).
//!
//! MCP names each revision by its date. The client sends the one it wants; the server answers with the same one if
//! it speaks it, and otherwise with its own newest, which the client may then refuse (MCP lifecycle, "Version
//! Negotiation"). Every revision listed here works for what this server offers, `tools/list` and `tools/call` with
//! one JSON reply each: the newer revisions only add fields (`title`, `outputSchema`, `structuredContent`) that an
//! older client ignores, and the HTTP transport differences stay between the bridge and the daemon.

/// Newest first; `pick_protocol_version` falls back to the first.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The client's revision when this server speaks it, else the newest one it does.
pub fn pick_protocol_version(requested: &str) -> &'static str {
    for version in SUPPORTED_PROTOCOL_VERSIONS {
        if *version == requested {
            return version;
        }
    }
    SUPPORTED_PROTOCOL_VERSIONS[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supported_revision_is_the_one_the_client_asked_for() {
        for version in SUPPORTED_PROTOCOL_VERSIONS {
            assert_eq!(pick_protocol_version(version), *version);
        }
    }

    #[test]
    fn an_unknown_revision_gets_our_newest() {
        assert_eq!(pick_protocol_version("2023-01-01"), "2025-11-25");
        assert_eq!(pick_protocol_version(""), "2025-11-25");
        assert_eq!(pick_protocol_version("2025-06-18 "), "2025-11-25");
    }

    #[test]
    fn revisions_are_listed_newest_first() {
        let mut sorted = SUPPORTED_PROTOCOL_VERSIONS.to_vec();
        sorted.sort();
        sorted.reverse();
        assert_eq!(sorted, SUPPORTED_PROTOCOL_VERSIONS);
    }
}
