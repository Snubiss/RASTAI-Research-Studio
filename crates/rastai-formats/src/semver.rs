//! In-house semver (spec §13, §20): plugin versions, the plugin API range in `engines.rastai`, `requires` in the
//! shared header, and plugin dependencies.
//!
//! Versions follow Semantic Versioning 2.0.0: `MAJOR.MINOR.PATCH`, an optional `-pre.release`, and optional
//! `+build` metadata, which is read and then ignored (semver says it doesn't affect precedence).
//!
//! A range is comparators separated by commas, all of which must match: `>=1.2, <2`. The comparators:
//! - `=1.2.3` and a bare `1.2.3`: exactly that version. A partial version is a wildcard: `1.2` means `1.2.x`.
//!   (A bare version means `=`, as in npm, whose `engines` field the manifest mirrors.)
//! - `>`, `>=`, `<`, `<=`: comparisons. With a partial version, `>1.2` means above every `1.2.x`, and `<=1.2`
//!   includes every `1.2.x`.
//! - `^1.2.3`: compatible updates, up to the next change of the first non-zero part (`^0.2.3` stays below 0.3.0).
//! - `~1.2.3`: patch updates only, below 1.3.0.
//! - `*`: any release.
//!
//! A pre-release version (`2.0.0-beta`) matches only when one of the range's comparators names a pre-release of
//! that same `MAJOR.MINOR.PATCH`, as in npm and Cargo: `<2` must not quietly accept `2.0.0-beta`.

use std::cmp::Ordering;

use crate::error::FormatsError;

const RANGE_SEPARATOR: char = ',';
const PART_SEPARATOR: char = '.';
const PRE_RELEASE_MARK: char = '-';
const BUILD_MARK: char = '+';
const ANY_VERSION: &str = "*";

// Why a version or range was refused; carried in `FormatsError::InvalidVersion.reason`.
const EMPTY_RANGE: &str = "the range is empty";
const EMPTY_COMPARATOR: &str = "a comparator between commas is empty";
const NOT_A_NUMBER: &str = "a version part is not a number without leading zeros";
const TOO_MANY_PARTS: &str = "a version has at most three numbers: MAJOR.MINOR.PATCH";
const MISSING_PARTS: &str = "a version needs all three numbers: MAJOR.MINOR.PATCH";
const BAD_IDENTIFIER: &str = "a pre-release or build part is empty or has a character outside [0-9A-Za-z-]";
const PARTIAL_PRE_RELEASE: &str = "a pre-release needs all three numbers before it";

/// A version. `pre_release` is the text after `-` ("rc.1"), empty for a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre_release: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Exact,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Caret,
    Tilde,
    Any,
}

/// A version in a range, where the minor and patch numbers may be left out.
#[derive(Debug, Clone)]
struct PartialVersion {
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
    pre_release: String,
}

#[derive(Debug, Clone)]
struct Comparator {
    operator: Operator,
    version: PartialVersion,
}

/// One end of the versions a comparator allows.
struct Bound {
    version: Version,
    inclusive: bool,
}

/// Reads a full version: `1.2.3`, `1.2.3-rc.1`, `1.2.3+build.5`.
pub fn parse_version(text: &str) -> Result<Version, FormatsError> {
    let partial = parse_partial_version(text.trim()).map_err(|reason| invalid(text, reason))?;
    let (Some(minor), Some(patch)) = (partial.minor, partial.patch) else {
        return Err(invalid(text, MISSING_PARTS));
    };
    Ok(Version {
        major: partial.major,
        minor,
        patch,
        pre_release: partial.pre_release,
    })
}

/// Semver precedence: numbers first, then a pre-release sorts before its release, then pre-release identifiers
/// one by one (numbers numerically and below words; a shorter list first when the rest is equal).
pub fn compare_versions(left: &Version, right: &Version) -> Ordering {
    let numbers = (left.major, left.minor, left.patch).cmp(&(right.major, right.minor, right.patch));
    if numbers != Ordering::Equal {
        return numbers;
    }
    match (left.pre_release.is_empty(), right.pre_release.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => compare_pre_releases(&left.pre_release, &right.pre_release),
    }
}

/// Whether a version is in a range such as `">=1.2, <2"`, `"^1.2"` or `"=1.2.3"`.
pub fn version_matches(range: &str, version: &Version) -> Result<bool, FormatsError> {
    let comparators = parse_range(range).map_err(|reason| invalid(range, reason))?;
    if !version.pre_release.is_empty() && !names_pre_release_of(&comparators, version) {
        return Ok(false);
    }
    Ok(comparators
        .iter()
        .all(|comparator| comparator_matches(comparator, version)))
}

fn invalid(text: &str, reason: &'static str) -> FormatsError {
    FormatsError::InvalidVersion {
        text: text.to_string(),
        reason,
    }
}

fn parse_range(range: &str) -> Result<Vec<Comparator>, &'static str> {
    if range.trim().is_empty() {
        return Err(EMPTY_RANGE);
    }
    range
        .split(RANGE_SEPARATOR)
        .map(|part| parse_comparator(part.trim()))
        .collect()
}

fn parse_comparator(text: &str) -> Result<Comparator, &'static str> {
    if text.is_empty() {
        return Err(EMPTY_COMPARATOR);
    }
    if text == ANY_VERSION {
        let version = PartialVersion {
            major: 0,
            minor: None,
            patch: None,
            pre_release: String::new(),
        };
        return Ok(Comparator {
            operator: Operator::Any,
            version,
        });
    }
    let (operator, rest) = split_operator(text);
    let version = parse_partial_version(rest.trim_start())?;
    Ok(Comparator { operator, version })
}

/// The operator at the start of a comparator; two-character operators are tried first.
fn split_operator(text: &str) -> (Operator, &str) {
    let operators = [
        (">=", Operator::GreaterOrEqual),
        ("<=", Operator::LessOrEqual),
        (">", Operator::Greater),
        ("<", Operator::Less),
        ("=", Operator::Exact),
        ("^", Operator::Caret),
        ("~", Operator::Tilde),
    ];
    for (symbol, operator) in operators {
        if let Some(rest) = text.strip_prefix(symbol) {
            return (operator, rest);
        }
    }
    (Operator::Exact, text)
}

/// `MAJOR[.MINOR[.PATCH[-pre]]][+build]`.
fn parse_partial_version(text: &str) -> Result<PartialVersion, &'static str> {
    let (without_build, build) = split_off(text, BUILD_MARK);
    if let Some(build) = build {
        check_identifiers(build, false)?;
    }
    let (core, pre_release) = split_off(without_build, PRE_RELEASE_MARK);
    let numbers = core
        .split(PART_SEPARATOR)
        .map(parse_number)
        .collect::<Result<Vec<u64>, &'static str>>()?;
    if numbers.len() > 3 {
        return Err(TOO_MANY_PARTS);
    }
    if let Some(pre_release) = pre_release {
        if numbers.len() < 3 {
            return Err(PARTIAL_PRE_RELEASE);
        }
        check_identifiers(pre_release, true)?;
    }
    Ok(PartialVersion {
        major: numbers[0],
        minor: numbers.get(1).copied(),
        patch: numbers.get(2).copied(),
        pre_release: pre_release.unwrap_or("").to_string(),
    })
}

/// Splits at the first `mark`: `"1.2.3-rc"` at `-` → `("1.2.3", Some("rc"))`.
fn split_off(text: &str, mark: char) -> (&str, Option<&str>) {
    match text.split_once(mark) {
        Some((before, after)) => (before, Some(after)),
        None => (text, None),
    }
}

/// Digits only, no leading zero, fits in u64.
fn parse_number(text: &str) -> Result<u64, &'static str> {
    let all_digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !all_digits || (text.len() > 1 && text.starts_with('0')) {
        return Err(NOT_A_NUMBER);
    }
    text.parse().map_err(|_| NOT_A_NUMBER)
}

/// Dot-separated identifiers of `[0-9A-Za-z-]`. In a pre-release a numeric identifier has no leading zero.
fn check_identifiers(text: &str, is_pre_release: bool) -> Result<(), &'static str> {
    for identifier in text.split(PART_SEPARATOR) {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-';
        if identifier.is_empty() || !identifier.bytes().all(allowed) {
            return Err(BAD_IDENTIFIER);
        }
        let numeric = identifier.bytes().all(|byte| byte.is_ascii_digit());
        if is_pre_release && numeric && parse_number(identifier).is_err() {
            return Err(NOT_A_NUMBER);
        }
    }
    Ok(())
}

fn compare_pre_releases(left: &str, right: &str) -> Ordering {
    let mut left_parts = left.split(PART_SEPARATOR);
    let mut right_parts = right.split(PART_SEPARATOR);
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_part), Some(right_part)) => {
                let order = compare_identifiers(left_part, right_part);
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// Numbers compare numerically and sort before words; words compare in ASCII order.
fn compare_identifiers(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left_number), Ok(right_number)) => left_number.cmp(&right_number),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

/// True when a comparator names a pre-release of the same `MAJOR.MINOR.PATCH` as the version.
fn names_pre_release_of(comparators: &[Comparator], version: &Version) -> bool {
    comparators.iter().any(|comparator| {
        let named = &comparator.version;
        !named.pre_release.is_empty()
            && named.major == version.major
            && named.minor == Some(version.minor)
            && named.patch == Some(version.patch)
    })
}

fn comparator_matches(comparator: &Comparator, version: &Version) -> bool {
    let (lower, upper) = comparator_bounds(comparator);
    let above_lower = lower.is_none_or(|bound| is_above(version, &bound));
    let below_upper = upper.is_none_or(|bound| is_below(version, &bound));
    above_lower && below_upper
}

fn is_above(version: &Version, bound: &Bound) -> bool {
    match compare_versions(version, &bound.version) {
        Ordering::Greater => true,
        Ordering::Equal => bound.inclusive,
        Ordering::Less => false,
    }
}

fn is_below(version: &Version, bound: &Bound) -> bool {
    match compare_versions(version, &bound.version) {
        Ordering::Less => true,
        Ordering::Equal => bound.inclusive,
        Ordering::Greater => false,
    }
}

/// The lowest and highest version a comparator allows; None is unbounded.
fn comparator_bounds(comparator: &Comparator) -> (Option<Bound>, Option<Bound>) {
    let partial = &comparator.version;
    let low = filled_version(partial);
    let is_full = partial.patch.is_some();
    let inclusive = |version| {
        Some(Bound {
            version,
            inclusive: true,
        })
    };
    let exclusive = |version| {
        Some(Bound {
            version,
            inclusive: false,
        })
    };
    match comparator.operator {
        Operator::Any => (None, None),
        Operator::GreaterOrEqual => (inclusive(low), None),
        Operator::Less => (None, exclusive(low)),
        Operator::Exact if is_full => (inclusive(low.clone()), inclusive(low)),
        Operator::Greater if is_full => (exclusive(low), None),
        Operator::LessOrEqual if is_full => (None, inclusive(low)),
        Operator::Exact => (inclusive(low), wildcard_end(partial).and_then(exclusive)),
        Operator::Greater => (wildcard_end(partial).and_then(inclusive), None),
        Operator::LessOrEqual => (None, wildcard_end(partial).and_then(exclusive)),
        Operator::Caret => (inclusive(low), caret_end(partial).and_then(exclusive)),
        Operator::Tilde => (inclusive(low), tilde_end(partial).and_then(exclusive)),
    }
}

/// A partial version with its missing numbers as zero: `1.2` → `1.2.0`.
fn filled_version(partial: &PartialVersion) -> Version {
    Version {
        major: partial.major,
        minor: partial.minor.unwrap_or(0),
        patch: partial.patch.unwrap_or(0),
        pre_release: partial.pre_release.clone(),
    }
}

/// The first version past a partial version's wildcard: `1.2` → `1.3.0`, `1` → `2.0.0`. None at u64::MAX.
fn wildcard_end(partial: &PartialVersion) -> Option<Version> {
    match partial.minor {
        Some(minor) => release(partial.major, minor.checked_add(1)?, 0),
        None => release(partial.major.checked_add(1)?, 0, 0),
    }
}

/// `^`: below the next change of the first non-zero part. `^1.2.3` → `2.0.0`, `^0.2.3` → `0.3.0`,
/// `^0.0.3` → `0.0.4`; a left-out part counts as a wildcard, so `^0` → `1.0.0` and `^0.0` → `0.1.0`.
fn caret_end(partial: &PartialVersion) -> Option<Version> {
    match (partial.major, partial.minor, partial.patch) {
        (0, Some(0), Some(patch)) => release(0, 0, patch.checked_add(1)?),
        (0, Some(minor), _) => release(0, minor.checked_add(1)?, 0),
        (major, _, _) => release(major.checked_add(1)?, 0, 0),
    }
}

/// `~`: below the next minor version when the minor is given (`~1.2.3` → `1.3.0`), else the next major.
fn tilde_end(partial: &PartialVersion) -> Option<Version> {
    match partial.minor {
        Some(minor) => release(partial.major, minor.checked_add(1)?, 0),
        None => release(partial.major.checked_add(1)?, 0, 0),
    }
}

fn release(major: u64, minor: u64, patch: u64) -> Option<Version> {
    Some(Version {
        major,
        minor,
        patch,
        pre_release: String::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        parse_version(text).unwrap()
    }

    fn matches(range: &str, text: &str) -> bool {
        version_matches(range, &version(text)).unwrap()
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            version("1.22.333-rc.1+build.7"),
            Version {
                major: 1,
                minor: 22,
                patch: 333,
                pre_release: "rc.1".to_string()
            }
        );
        assert_eq!(version("0.0.0").pre_release, "");
        assert_eq!(version("1.0.0-x-y.0a").pre_release, "x-y.0a");
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "v1.2.3",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3-ä",
            "-1.2.3",
            "1.2.x",
            "18446744073709551616.0.0",
        ] {
            let error = parse_version(text).unwrap_err();
            assert_eq!(error.code(), "RRS-E1004", "{text:?}");
        }
    }

    // The precedence example of semver.org section 11, each version below the next.
    #[test]
    fn precedence_follows_the_spec() {
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
            "10.0.0",
        ];
        for pair in ordered.windows(2) {
            assert_eq!(
                compare_versions(&version(pair[0]), &version(pair[1])),
                Ordering::Less,
                "{pair:?}"
            );
            assert_eq!(
                compare_versions(&version(pair[1]), &version(pair[0])),
                Ordering::Greater,
                "{pair:?}"
            );
        }
        // Build metadata doesn't count.
        assert_eq!(
            compare_versions(&version("1.0.0+a"), &version("1.0.0+b")),
            Ordering::Equal
        );
    }

    #[test]
    fn comparison_ranges() {
        let range = ">=1.2, <2";
        assert!(matches(range, "1.2.0"));
        assert!(matches(range, "1.99.99"));
        assert!(!matches(range, "1.1.9"));
        assert!(!matches(range, "2.0.0"));
        assert!(matches(">1.2", "1.3.0"));
        assert!(!matches(">1.2", "1.2.9"));
        assert!(matches(">1.2.3", "1.2.4"));
        assert!(!matches(">1.2.3", "1.2.3"));
        assert!(matches("<=1.2", "1.2.9"));
        assert!(!matches("<=1.2", "1.3.0"));
        assert!(matches("<=1.2.3", "1.2.3"));
        assert!(matches(">= 1.0.0 ,  < 1.0.1", "1.0.0"));
    }

    #[test]
    fn exact_and_wildcard_ranges() {
        assert!(matches("=1.2.3", "1.2.3"));
        assert!(!matches("=1.2.3", "1.2.4"));
        assert!(matches("1.2.3", "1.2.3"));
        assert!(matches("1.2", "1.2.7"));
        assert!(!matches("1.2", "1.3.0"));
        assert!(matches("=1", "1.9.0"));
        assert!(!matches("=1", "2.0.0"));
        assert!(matches("*", "0.0.1"));
        assert!(matches("*", "99.0.0"));
    }

    #[test]
    fn caret_and_tilde_ranges() {
        let cases = [
            ("^1.2", "1.2.0", true),
            ("^1.2", "1.99.0", true),
            ("^1.2", "2.0.0", false),
            ("^1.2", "1.1.9", false),
            ("^1.2.3", "1.2.2", false),
            ("^0.2.3", "0.2.9", true),
            ("^0.2.3", "0.3.0", false),
            ("^0.0.3", "0.0.3", true),
            ("^0.0.3", "0.0.4", false),
            ("^0", "0.9.9", true),
            ("^0", "1.0.0", false),
            ("^0.0", "0.0.9", true),
            ("^0.0", "0.1.0", false),
            ("~1.2.3", "1.2.9", true),
            ("~1.2.3", "1.3.0", false),
            ("~1.2.3", "1.2.2", false),
            ("~1", "1.9.0", true),
            ("~1", "2.0.0", false),
        ];
        for (range, text, expected) in cases {
            assert_eq!(matches(range, text), expected, "{range} with {text}");
        }
    }

    #[test]
    fn pre_releases_match_only_when_named() {
        assert!(!matches("<2", "2.0.0-beta"));
        assert!(!matches(">=1.2, <2", "1.5.0-rc.1"));
        assert!(matches(">=1.0.0-beta", "1.0.0-rc.1"));
        assert!(matches(">=1.0.0-beta", "1.0.0"));
        assert!(!matches(">=1.0.0-beta", "1.1.0-alpha"));
        assert!(!matches(">=1.0.0-beta", "1.0.0-alpha"));
        assert!(matches("=1.0.0-beta", "1.0.0-beta"));
        assert!(!matches("*", "1.0.0-beta"));
    }

    #[test]
    fn bad_ranges_are_refused() {
        let release = version("1.0.0");
        for range in [
            "",
            " ",
            ",",
            ">=1.2,",
            ">=",
            "^",
            "1.2.3.4",
            "01.2",
            ">= 1.2 <2",
            "^1.2-beta",
            "=>1.2",
            "x",
        ] {
            let error = version_matches(range, &release).unwrap_err();
            assert_eq!(error.code(), "RRS-E1004", "{range:?}");
        }
    }

    #[test]
    fn largest_numbers_do_not_overflow() {
        let top = format!("{}.{}.{}", u64::MAX, u64::MAX, u64::MAX);
        assert!(matches(&format!("^{top}"), &top));
        assert!(matches(&format!("~{}", u64::MAX), &top));
        assert!(matches(&format!("{}", u64::MAX), &top));
    }
}
