//! Who owns which codes: each catalog prefix, the file that holds its entries, and its number ranges, copied from
//! contracts 10.1. The catalog checks (tests/catalog_checks.rs) refuse a code outside its owner's ranges, and
//! docs/reference/errors.md uses the areas as headings.
//!
//! When contracts 10.1 gains a range, this table gains it in the same PR: a catalog check reads the contracts table
//! and fails while the two differ.

/// The code format: "RRS-", one capital letter, four digits ("RRS-E1003"), contracts 10.
const CODE_START: &str = "RRS-";
const CODE_DIGIT_COUNT: usize = 4;
/// "RRS-E": the start plus the letter.
const PREFIX_LENGTH: usize = CODE_START.len() + 1;

/// One range of numbers inside a prefix, both ends included, with what its codes are about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeRange {
    pub first: u32,
    pub last: u32,
    pub area: &'static str,
}

/// One prefix, its owner's file and its ranges (one row of contracts 10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogPrefix {
    pub prefix: &'static str,
    pub area: &'static str,
    /// The only file that may hold this prefix's entries, relative to the repository root.
    pub source_file: &'static str,
    pub ranges: &'static [CodeRange],
}

const fn range(first: u32, last: u32, area: &'static str) -> CodeRange {
    CodeRange { first, last, area }
}

/// Contracts 10.1, in its order. RRS-L and RRS-V live in the first-party plugins' own catalogs.
pub const CATALOG_PREFIXES: &[CatalogPrefix] = &[
    CatalogPrefix {
        prefix: "RRS-E",
        area: "Formats and validation",
        source_file: "catalog/errors/RRS-E.json",
        ranges: &[
            range(1, 999, "syntax"),
            range(1000, 1999, "structure"),
            range(2000, 2999, "meaning (shapes, references, ranges, cycles)"),
            range(3000, 3999, "environment warnings"),
            range(4000, 4999, "versions and migrations"),
            range(5000, 5999, "untrusted-import limits (size, depth, zip bomb, zip slip)"),
            range(6000, 6499, "safetensors (strict contract)"),
            range(6500, 6999, "GGUF"),
            range(7000, 7999, "importers (HF config, torch.export, ONNX, conversations)"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-P",
        area: "Plugins",
        source_file: "catalog/errors/RRS-P.json",
        ranges: &[
            range(1, 999, "manifest"),
            range(1000, 1999, "engines and capability negotiation"),
            range(2000, 2999, "permissions, trust and isolation"),
            range(3000, 3999, "install, dependencies, lockfiles, plugin data"),
            range(4000, 4099, "plugin module runtime"),
            range(4100, 4199, "iframe bridge and plugin UI"),
            range(5000, 5999, "SDK tooling (new, dev, test, lint, pack, codegen)"),
            range(6000, 6999, "registry and signing (Phase 2)"),
            range(7000, 7999, "licensing"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-C",
        area: "Commands and API",
        source_file: "catalog/errors/RRS-C.json",
        ranges: &[
            range(1, 399, "dispatch, arguments and settings files"),
            range(400, 499, "event stream"),
            range(1000, 1999, "auth, scopes, Host/Origin, sessions, data policy"),
            range(2000, 2999, "undo"),
            range(3000, 3999, "MCP"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-S",
        area: "Storage and durability",
        source_file: "catalog/errors/RRS-S.json",
        ranges: &[
            range(1, 999, "databases"),
            range(1000, 1999, "blob store"),
            range(2000, 2999, "event logs"),
            range(3000, 3999, "disk, quotas, GC, snapshots"),
            range(4000, 4999, "secrets and keychain"),
            range(5000, 5999, "long-term archive"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-N",
        area: "Nodes, transport, remote",
        source_file: "catalog/errors/RRS-N.json",
        ranges: &[
            range(1, 999, "handshake and versions"),
            range(1000, 1999, "pairing and SSH"),
            range(2000, 2999, "heartbeat and reachability"),
            range(3000, 3999, "downloads and transfers"),
            range(4000, 4999, "self-update"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-H",
        area: "Hardware and environments",
        source_file: "catalog/errors/RRS-H.json",
        ranges: &[
            range(1, 999, "detection"),
            range(1000, 1999, "numerical self-test"),
            range(2000, 2999, "uv environments"),
            range(3000, 3999, "VRAM leases and OOM"),
            range(4000, 4999, "drivers and backends"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-J",
        area: "Jobs and worker protocol",
        source_file: "catalog/errors/RRS-J.json",
        ranges: &[
            range(1, 999, "lifecycle"),
            range(1000, 1999, "crash and exit causes"),
            range(2000, 2999, "worker protocol"),
            range(3000, 3999, "reattach"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-G",
        area: "Gateway and inference",
        source_file: "catalog/errors/RRS-G.json",
        ranges: &[
            range(1, 999, "endpoints and requests"),
            range(1000, 1999, "tokenization and chat templates"),
            range(2000, 2999, "API adapters"),
            range(3000, 3999, "Fast runtime (llama-server) and twins"),
            range(4000, 4999, "logging, retention, incognito"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-M",
        area: "Model runtime",
        source_file: "catalog/errors/RRS-M.json",
        ranges: &[
            // 0600–0699 inside it are Eject and ONNX.
            range(1, 999, "compiler"),
            range(1000, 1999, "families and import mapping"),
            range(2000, 2999, "weight loading"),
            range(3000, 3999, "parity"),
            range(4000, 4999, "decode and CUDA graphs"),
            range(5000, 5999, "placement across GPUs"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-T",
        area: "Training",
        source_file: "catalog/errors/RRS-T.json",
        ranges: &[
            // 0200–0299 inside it are the RLHF gate.
            range(1, 999, "recipe config"),
            range(1000, 1999, "data loading"),
            range(2000, 2999, "checkpoint and resume"),
            range(3000, 3999, "numerical (NaN, divergence)"),
            range(4000, 4999, "export"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-D",
        area: "Data",
        source_file: "catalog/errors/RRS-D.json",
        ranges: &[
            range(1, 999, "task detection"),
            range(1000, 1999, "converters"),
            range(2000, 2999, "checks (dedupe, leakage, PII)"),
            range(3000, 3999, "sources (HF, URL, MySQL)"),
            range(4000, 4999, "run-log import"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-R",
        area: "Agent Gym and sandboxes",
        source_file: "catalog/errors/RRS-R.json",
        ranges: &[
            range(1, 999, "harness"),
            range(1000, 1999, "sandbox manager"),
            range(2000, 2999, "rollouts"),
            range(3000, 3999, "MCP tools"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-L",
        area: "Psychology plugin",
        source_file: "plugins/psychology/catalog/errors.json",
        ranges: &[
            range(1, 999, "replay"),
            range(1000, 1999, "live panel"),
            range(2000, 2999, "SAEs"),
            range(3000, 3999, "interventions"),
            range(4000, 4999, "offline analyses"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-V",
        area: "Evaluation plugin",
        source_file: "plugins/evaluation/catalog/errors.json",
        ranges: &[
            range(1, 999, "suites and tests"),
            range(1000, 1999, "judges"),
            range(2000, 2999, "metrics"),
            range(3000, 3999, "benchmarks"),
        ],
    },
    CatalogPrefix {
        prefix: "RRS-U",
        area: "UI client and desktop",
        source_file: "catalog/errors/RRS-U.json",
        ranges: &[
            range(1, 99, "connection and session"),
            range(100, 199, "desktop shell"),
            range(1000, 1999, "rendering"),
        ],
    },
];

/// Splits a well-formed code into its prefix and number: "RRS-E1003" is ("RRS-E", 1003). None when the code isn't
/// "RRS-", one capital letter and four digits.
pub fn split_code(code: &str) -> Option<(&str, u32)> {
    if !code.is_ascii() || code.len() != PREFIX_LENGTH + CODE_DIGIT_COUNT || !code.starts_with(CODE_START) {
        return None;
    }
    let (prefix, digits) = code.split_at(PREFIX_LENGTH);
    let letter_is_capital = prefix.chars().last().is_some_and(|letter| letter.is_ascii_uppercase());
    let digits_are_digits = digits.chars().all(|digit| digit.is_ascii_digit());
    if !letter_is_capital || !digits_are_digits {
        return None;
    }
    let number = digits.parse::<u32>().ok()?;
    Some((prefix, number))
}

/// The row of contracts 10.1 for a prefix ("RRS-E"), or None for a prefix nobody owns.
pub fn find_prefix(prefix: &str) -> Option<&'static CatalogPrefix> {
    CATALOG_PREFIXES.iter().find(|row| row.prefix == prefix)
}

/// The range a number falls in, or None when it is in none of the prefix's ranges.
pub fn find_range(row: &CatalogPrefix, number: u32) -> Option<&'static CodeRange> {
    row.ranges
        .iter()
        .find(|candidate| candidate.first <= number && number <= candidate.last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_codes_split_into_prefix_and_number() {
        assert_eq!(split_code("RRS-E1003"), Some(("RRS-E", 1003)));
        assert_eq!(split_code("RRS-U0001"), Some(("RRS-U", 1)));
    }

    #[test]
    fn malformed_codes_do_not_split() {
        for code in [
            "",
            "RRS-E",
            "RRS-E100",
            "RRS-E10030",
            "RRS-e1003",
            "RRS-11003",
            "RRS-E10a3",
            "rrs-E1003",
        ] {
            assert_eq!(split_code(code), None, "{code}");
        }
        assert_eq!(split_code("PLG-E1003"), None);
        assert_eq!(split_code("RRS-É103"), None);
        assert_eq!(split_code("RRS-E+103"), None);
    }

    #[test]
    fn every_prefix_is_listed_once_with_ranges_in_order() {
        for (index, row) in CATALOG_PREFIXES.iter().enumerate() {
            assert!(
                CATALOG_PREFIXES[index + 1..]
                    .iter()
                    .all(|other| other.prefix != row.prefix)
            );
            assert!(!row.ranges.is_empty(), "{}", row.prefix);
            for pair in row.ranges.windows(2) {
                assert!(
                    pair[0].last < pair[1].first,
                    "{} ranges overlap or are out of order",
                    row.prefix
                );
            }
            assert!(
                row.ranges
                    .iter()
                    .all(|item| 1 <= item.first && item.first <= item.last && item.last <= 9999)
            );
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn numbers_between_ranges_belong_to_none() {
        let commands = find_prefix("RRS-C").unwrap();
        assert_eq!(
            find_range(commands, 399).unwrap().area,
            "dispatch, arguments and settings files"
        );
        assert_eq!(find_range(commands, 400).unwrap().area, "event stream");
        assert_eq!(find_range(commands, 999), None);
        assert_eq!(find_range(commands, 0), None);
        assert_eq!(find_prefix("RRS-Z"), None);
    }
}
