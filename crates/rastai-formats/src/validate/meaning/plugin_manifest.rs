//! Layer 3 for `rastai.plugin/1`, and the id rules of contracts 6.1 it shares with rastai-core's command registry.
//!
//! The rules are written once, here, because rastai-formats sits below rastai-core: the manifest check and the
//! registry (`check_command_id`, the `plugin:<plugin_id>` scope) call the same functions.
//! - A plugin id is one id segment: lowercase words of letters and digits joined by single underscores, starting with
//!   a letter (`acme_tools2`; never `acme_`, `my__tools` or `2fast`), and not a reserved core area.
//! - A command id is two or three such segments: `<area>.<verb>` or `<area>.<noun>.<verb>`.
//! - A plugin's commands start with its own id.
//!
//! What the manifest check reports, in the order the fields are written:
//! - a malformed plugin id: `RRS-E2006` at `/plugin_id`; a core area as the id: `RRS-P0002`. Either way its commands
//!   aren't checked further, because every one of them would repeat the same problem;
//! - a malformed command id: `RRS-E2006`; one outside the plugin's namespace: `RRS-P0004`; one listed twice:
//!   `RRS-E2005` at the second;
//! - an event handler whose command isn't one of the plugin's own: `RRS-P0004` (layout 12.4);
//! - a non-empty `default_shortcut` that doesn't parse, or has neither Mod nor Alt: `RRS-E2006` at
//!   `/contributes/commands/<i>/default_shortcut`. The grammar mirrors apps/web/src/commands/keybindings.js
//!   (`parseShortcut`), which the web app reads the same text with, so a shortcut one side refuses the other refuses too.

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::plugin_manifest::PluginManifest;
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_DUPLICATE_ID: &str = "RRS-E2005";
const CODE_VALUE_FORM: &str = "RRS-E2006";
const CODE_PLUGIN_ID_RESERVED: &str = "RRS-P0002";
const CODE_COMMAND_OUTSIDE_NAMESPACE: &str = "RRS-P0004";

const PLUGIN_ID_POINTER: &str = "/plugin_id";
const COMMANDS_POINTER: &str = "/contributes/commands";
const EVENT_HANDLERS_POINTER: &str = "/contributes/event_handlers";
const COMMAND_ID_KEY: &str = "command_id";
const DEFAULT_SHORTCUT_KEY: &str = "default_shortcut";
const EXPECTED_COMMAND_ID: &str = "<plugin_id>.<verb> or <plugin_id>.<noun>.<verb>, in lowercase snake_case";
const EXPECTED_DEFAULT_SHORTCUT: &str =
    "a shortcut with Mod or Alt: modifiers Mod, Shift or Alt joined by +, then one key, such as Mod+Shift+P or Alt+F";

const ID_SEPARATOR: char = '.';
const WORD_SEPARATOR: char = '_';
const MIN_COMMAND_SEGMENTS: usize = 2;
const MAX_COMMAND_SEGMENTS: usize = 3;

// The shortcut grammar, mirrored from apps/web/src/commands/keybindings.js.
const SHORTCUT_PART_SEPARATOR: char = '+';
const SHORTCUT_MODIFIER_MOD: &str = "mod";
const SHORTCUT_MODIFIER_SHIFT: &str = "shift";
const SHORTCUT_MODIFIER_ALT: &str = "alt";

/// What a plugin id should look like, for the manifest check here and the registry's `check_plugin_id`.
pub const EXPECTED_PLUGIN_ID_FORM: &str =
    "a plugin id: lowercase letters and digits in words joined by single underscores, starting with a letter";

/// The reserved core areas (contracts 6.1). No plugin may use one as its id, and every core command sits in one.
/// `view` holds UI-only actions and `core` is the pseudo plugin id of core sections and panels.
pub const CORE_AREAS: &[&str] = &[
    "system",
    "settings",
    "token",
    "secret",
    "storage",
    "archive",
    "artifact",
    "project",
    "file",
    "document",
    "workspace",
    "command",
    "job",
    "hf",
    "model",
    "chat",
    "conversation",
    "adapter",
    "gateway",
    "train",
    "teach",
    "rlhf",
    "sweep",
    "recipe",
    "run",
    "checkpoint",
    "design",
    "harness",
    "gym",
    "sandbox",
    "plugin",
    "node",
    "transfer",
    "dataset",
    "converter",
    "tutorial",
    "view",
    "core",
];

/// Every meaning problem of a plugin manifest.
pub fn check_plugin_manifest(manifest: &PluginManifest, source: &MeaningSource) -> Vec<Diagnostic> {
    let plugin_id = manifest.plugin_id.as_str();
    if !is_plugin_id(plugin_id) {
        let values = [("found", shown_value(plugin_id)), ("expected", EXPECTED_PLUGIN_ID_FORM)];
        return vec![meaning_diagnostic(CODE_VALUE_FORM, &values, source, PLUGIN_ID_POINTER)];
    }
    if is_core_area(plugin_id) {
        let values = [("plugin_id", plugin_id)];
        return vec![meaning_diagnostic(
            CODE_PLUGIN_ID_RESERVED,
            &values,
            source,
            PLUGIN_ID_POINTER,
        )];
    }
    let mut diagnostics = check_commands(manifest, source);
    diagnostics.extend(check_event_handlers(manifest, source));
    diagnostics
}

fn check_commands(manifest: &PluginManifest, source: &MeaningSource) -> Vec<Diagnostic> {
    let plugin_id = manifest.plugin_id.as_str();
    let mut diagnostics = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (index, command) in manifest.contributes.commands.iter().enumerate() {
        let command_id = command.command_id.as_str();
        let pointer = format!("{COMMANDS_POINTER}/{index}/{COMMAND_ID_KEY}");
        if !is_command_id(command_id) {
            let values = [("found", shown_value(command_id)), ("expected", EXPECTED_COMMAND_ID)];
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        } else if command_area(command_id) != plugin_id {
            let values = [("command_id", command_id), ("plugin_id", plugin_id)];
            diagnostics.push(meaning_diagnostic(
                CODE_COMMAND_OUTSIDE_NAMESPACE,
                &values,
                source,
                &pointer,
            ));
        } else if seen.contains(&command_id) {
            let values = [("id", command_id)];
            diagnostics.push(meaning_diagnostic(CODE_DUPLICATE_ID, &values, source, &pointer));
        }
        seen.push(command_id);
        let shortcut = command.default_shortcut.as_str();
        if !shortcut.is_empty() && !is_usable_shortcut(shortcut) {
            let values = [
                ("found", shown_value(shortcut)),
                ("expected", EXPECTED_DEFAULT_SHORTCUT),
            ];
            let pointer = format!("{COMMANDS_POINTER}/{index}/{DEFAULT_SHORTCUT_KEY}");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
    }
    diagnostics
}

/// A default shortcut the web app can bind: its parts split on `+`, each trimmed and lowercased, with the empty parts
/// dropped. Every part but the last is Mod, Shift or Alt, and the last is the key. It needs Mod or Alt, so a bare key
/// (`Enter`, `x`) or a Shift-only key (`Shift+K`) is refused. `Ctrl` isn't a modifier word here, so `Ctrl+K` is refused.
pub fn is_usable_shortcut(text: &str) -> bool {
    let parts: Vec<String> = text
        .split(SHORTCUT_PART_SEPARATOR)
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    let Some((_key, modifiers)) = parts.split_last() else {
        return false;
    };
    let all_known = modifiers.iter().all(|modifier| {
        modifier == SHORTCUT_MODIFIER_MOD || modifier == SHORTCUT_MODIFIER_SHIFT || modifier == SHORTCUT_MODIFIER_ALT
    });
    let has_mod_or_alt = modifiers
        .iter()
        .any(|modifier| modifier == SHORTCUT_MODIFIER_MOD || modifier == SHORTCUT_MODIFIER_ALT);
    all_known && has_mod_or_alt
}

/// A handler runs as the plugin, so it may only name the plugin's own commands.
fn check_event_handlers(manifest: &PluginManifest, source: &MeaningSource) -> Vec<Diagnostic> {
    let plugin_id = manifest.plugin_id.as_str();
    let commands = &manifest.contributes.commands;
    let mut diagnostics = Vec::new();
    for (index, handler) in manifest.contributes.event_handlers.iter().enumerate() {
        let command_id = handler.command_id.as_str();
        let own_command = commands.iter().any(|command| command.command_id == command_id);
        if !own_command {
            let values = [("command_id", command_id), ("plugin_id", plugin_id)];
            let pointer = format!("{EVENT_HANDLERS_POINTER}/{index}/{COMMAND_ID_KEY}");
            diagnostics.push(meaning_diagnostic(
                CODE_COMMAND_OUTSIDE_NAMESPACE,
                &values,
                source,
                &pointer,
            ));
        }
    }
    diagnostics
}

/// A plugin id: one id segment (contracts 6.1). The empty id core commands carry is not one.
pub fn is_plugin_id(text: &str) -> bool {
    is_id_segment(text)
}

/// `<area>.<verb>` or `<area>.<noun>.<verb>`, every segment an id segment.
pub fn is_command_id(text: &str) -> bool {
    let segments: Vec<&str> = text.split(ID_SEPARATOR).collect();
    let segment_count_ok = (MIN_COMMAND_SEGMENTS..=MAX_COMMAND_SEGMENTS).contains(&segments.len());
    segment_count_ok && segments.iter().all(|segment| is_id_segment(segment))
}

/// True for one of the reserved core areas.
pub fn is_core_area(area: &str) -> bool {
    CORE_AREAS.contains(&area)
}

/// Starts with a letter; `[a-z0-9]+` words joined by single underscores: `add_conversation`, `top_k2`, `a1_2b`; not
/// `_x`, `x_`, `a__b`, `1a`. This is what makes the CLI and MCP names lossless: `-` in a CLI word and `__` in an MCP
/// tool name can only come from one place in the id.
pub fn is_id_segment(segment: &str) -> bool {
    let starts_with_letter = segment.bytes().next().is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && segment.split(WORD_SEPARATOR).all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// The first segment of a well-formed command id.
fn command_area(command_id: &str) -> &str {
    command_id.split(ID_SEPARATOR).next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_segments_are_single_underscore_snake_case() {
        for segment in ["a", "acme", "acme_tools2", "a1_2b", "top_k"] {
            assert!(is_id_segment(segment), "{segment}");
        }
        for segment in ["", "_x", "x_", "a__b", "1a", "Acme", "acme-tools", "acme.tools"] {
            assert!(!is_id_segment(segment), "{segment}");
        }
    }

    #[test]
    fn command_ids_have_two_or_three_segments() {
        assert!(is_command_id("acme.run"));
        assert!(is_command_id("acme.report.export"));
        for command_id in ["acme", "acme.a.b.c", "acme..run", "acme.Run", ".run", "acme.run."] {
            assert!(!is_command_id(command_id), "{command_id}");
        }
    }

    #[test]
    fn core_areas_are_reserved() {
        assert!(is_core_area("model") && is_core_area("core") && is_core_area("view"));
        assert!(!is_core_area("psychology") && !is_core_area("eval"));
        assert_eq!(CORE_AREAS.len(), 38);
    }

    #[test]
    fn default_shortcuts_need_mod_or_alt_and_the_web_grammar() {
        for shortcut in ["Mod+Shift+P", "Alt+F", "mod+alt+x", " Mod + . ", "Alt+Shift+Enter"] {
            assert!(is_usable_shortcut(shortcut), "{shortcut}");
        }
        for shortcut in ["Enter", "Mod+", "Ctrl+K", "Shift+K", "Hyper+K", "x"] {
            assert!(!is_usable_shortcut(shortcut), "{shortcut}");
        }
    }

    #[test]
    fn an_empty_shortcut_is_left_for_the_caller_to_skip() {
        assert!(!is_usable_shortcut(""));
    }

    #[test]
    fn the_area_is_the_first_segment() {
        assert_eq!(command_area("acme.report.export"), "acme");
    }
}
