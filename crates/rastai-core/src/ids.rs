//! Generated ids (contracts 8.6): `<prefix>_<26 Crockford base32>`, for example `run_01JA2B3C4D5E6F7G8H9J0K1M2N`.
//!
//! The 16 bytes behind the 26 characters are a 48-bit big-endian Unix millisecond time followed by 80 random bits
//! (the ULID layout). The time comes first and the encoding keeps byte order, so ids of one kind sort by creation
//! time as plain text, which is what the indexes and `events.log` readers rely on. Two ids made in the same
//! millisecond are in random order.
//!
//! Node ids are different: `node_<52 Crockford base32>` of the node's Ed25519 public key, so the id proves which
//! key it names.

use crate::crockford::{decode_base32, encode_base32};
use crate::error::CoreError;
use crate::random::random_bytes;
use crate::time::now_unix_ms;

const ID_SEPARATOR: char = '_';
const ID_TIME_BYTES: usize = 6;
const ID_RANDOM_BYTES: usize = 10;
const ID_BYTES: usize = ID_TIME_BYTES + ID_RANDOM_BYTES;
// The largest time 48 bits hold: the year 10889, so no clamp is reached in practice.
const MAX_ID_TIME_UNIX_MS: i64 = (1 << 48) - 1;
const NODE_ID_PREFIX: &str = "node";

/// The kinds of generated id. Each has its own prefix (contracts 8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdPrefix {
    Run,
    Job,
    Checkpoint,
    Artifact,
    Conversation,
    Message,
    Request,
    Lease,
    Sandbox,
    Transfer,
    Project,
    Document,
    Journal,
    Token,
    /// A generation session.
    Session,
    Rollout,
    Group,
}

/// The prefix text written before the `_`.
pub fn id_prefix_text(prefix: IdPrefix) -> &'static str {
    match prefix {
        IdPrefix::Run => "run",
        IdPrefix::Job => "job",
        IdPrefix::Checkpoint => "ckpt",
        IdPrefix::Artifact => "art",
        IdPrefix::Conversation => "conv",
        IdPrefix::Message => "msg",
        IdPrefix::Request => "req",
        IdPrefix::Lease => "lease",
        IdPrefix::Sandbox => "sbx",
        IdPrefix::Transfer => "xfer",
        IdPrefix::Project => "prj",
        IdPrefix::Document => "doc",
        IdPrefix::Journal => "jrn",
        IdPrefix::Token => "tok",
        IdPrefix::Session => "ses",
        IdPrefix::Rollout => "ro",
        IdPrefix::Group => "grp",
    }
}

/// A new id of this kind, made now. Fails only when the operating system's random source can't be read.
pub fn new_id(prefix: IdPrefix) -> Result<String, CoreError> {
    let random = random_bytes(ID_RANDOM_BYTES)?;
    Ok(format_id(prefix, now_unix_ms(), &random))
}

/// True when `id` is a well-formed id of this kind: the prefix, `_`, and 26 canonical base32 characters. Ids read
/// from files and requests are checked with this before they name a folder or a row.
pub fn id_has_prefix(id: &str, prefix: IdPrefix) -> bool {
    match split_id(id, id_prefix_text(prefix)) {
        Some(body) => decode_base32(body, ID_BYTES).is_some(),
        None => false,
    }
}

/// `node_` and the base32 of the node's Ed25519 public key (contracts 8.6).
pub fn node_id_from_public_key(public_key: &[u8; 32]) -> String {
    format!("{NODE_ID_PREFIX}{ID_SEPARATOR}{}", encode_base32(public_key))
}

/// The id for a given time and random part. Times outside 48 bits are clamped, so a clock before 1970 gives ids
/// that sort first rather than wrapping around.
fn format_id(prefix: IdPrefix, unix_ms: i64, random: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(ID_BYTES);
    let time = unix_ms.clamp(0, MAX_ID_TIME_UNIX_MS).to_be_bytes();
    bytes.extend_from_slice(&time[time.len() - ID_TIME_BYTES..]);
    bytes.extend_from_slice(random);
    format!("{}{ID_SEPARATOR}{}", id_prefix_text(prefix), encode_base32(&bytes))
}

/// The base32 body after `<prefix>_`, or None when the id doesn't start with them.
fn split_id<'a>(id: &'a str, prefix_text: &str) -> Option<&'a str> {
    id.strip_prefix(prefix_text)?.strip_prefix(ID_SEPARATOR)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const ALL_PREFIXES: &[IdPrefix] = &[
        IdPrefix::Run,
        IdPrefix::Job,
        IdPrefix::Checkpoint,
        IdPrefix::Artifact,
        IdPrefix::Conversation,
        IdPrefix::Message,
        IdPrefix::Request,
        IdPrefix::Lease,
        IdPrefix::Sandbox,
        IdPrefix::Transfer,
        IdPrefix::Project,
        IdPrefix::Document,
        IdPrefix::Journal,
        IdPrefix::Token,
        IdPrefix::Session,
        IdPrefix::Rollout,
        IdPrefix::Group,
    ];
    // Contracts 8.6, in its order.
    const CONTRACT_PREFIXES: &[&str] = &[
        "run", "job", "ckpt", "art", "conv", "msg", "req", "lease", "sbx", "xfer", "prj", "doc", "jrn", "tok", "ses",
        "ro", "grp",
    ];
    // 2026-10-10T12:00:00Z.
    const NOON_UNIX_MS: i64 = 1_791_633_600_000;
    const ID_BODY_LENGTH: usize = 26;

    /// The time and random part an id was made from, read back through base32.
    fn decode_id(id: &str, prefix: IdPrefix) -> (i64, Vec<u8>) {
        let bytes = decode_base32(split_id(id, id_prefix_text(prefix)).unwrap(), ID_BYTES).unwrap();
        let mut time = [0u8; 8];
        time[8 - ID_TIME_BYTES..].copy_from_slice(&bytes[..ID_TIME_BYTES]);
        (i64::from_be_bytes(time), bytes[ID_TIME_BYTES..].to_vec())
    }

    #[test]
    fn prefixes_are_the_contract_ones() {
        let texts: Vec<&str> = ALL_PREFIXES.iter().map(|prefix| id_prefix_text(*prefix)).collect();
        assert_eq!(texts, CONTRACT_PREFIXES);
    }

    // The tests that call new_id are Unix only: other platforms have no random source yet (random.rs).
    #[cfg(unix)]
    #[test]
    fn new_ids_have_the_contract_form() {
        for prefix in ALL_PREFIXES {
            let id = new_id(*prefix).unwrap();
            let body = split_id(&id, id_prefix_text(*prefix)).unwrap();
            assert_eq!(body.len(), ID_BODY_LENGTH, "{id}");
            assert!(id_has_prefix(&id, *prefix), "{id}");
        }
    }

    #[test]
    fn ids_round_trip_through_base32() {
        let random = [0xA5, 0x00, 0xFF, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE];
        let id = format_id(IdPrefix::Run, NOON_UNIX_MS, &random);
        assert_eq!(
            split_id(&id, id_prefix_text(IdPrefix::Run)).unwrap().len(),
            ID_BODY_LENGTH
        );
        assert_eq!(decode_id(&id, IdPrefix::Run), (NOON_UNIX_MS, random.to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn new_ids_carry_the_time_they_were_made() {
        let made_now = new_id(IdPrefix::Job).unwrap();
        let (time, random_part) = decode_id(&made_now, IdPrefix::Job);
        assert!((time - now_unix_ms()).abs() < 60_000, "{made_now}");
        assert_eq!(random_part.len(), ID_RANDOM_BYTES);
    }

    // The random part must not reorder ids made at different times: the earlier id has the largest random part.
    #[test]
    fn ids_sort_by_creation_time() {
        let times = [
            0,
            1,
            999,
            NOON_UNIX_MS - 1,
            NOON_UNIX_MS,
            NOON_UNIX_MS + 1,
            MAX_ID_TIME_UNIX_MS,
        ];
        let ids: Vec<String> = times
            .iter()
            .enumerate()
            .map(|(index, time)| format_id(IdPrefix::Run, *time, &[0xFF - index as u8; ID_RANDOM_BYTES]))
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, ids);
    }

    #[cfg(unix)]
    #[test]
    fn ids_made_one_after_another_sort_in_order() {
        let first = new_id(IdPrefix::Run).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = new_id(IdPrefix::Run).unwrap();
        assert!(first < second, "{first} then {second}");
    }

    #[cfg(not(unix))]
    #[test]
    fn new_id_fails_without_a_random_source() {
        let error = new_id(IdPrefix::Run).unwrap_err();
        assert!(matches!(error, CoreError::RandomUnavailable { .. }), "{error:?}");
    }

    #[test]
    fn times_outside_48_bits_are_clamped() {
        let random = [0u8; ID_RANDOM_BYTES];
        assert_eq!(decode_id(&format_id(IdPrefix::Run, -5, &random), IdPrefix::Run).0, 0);
        let late = format_id(IdPrefix::Run, i64::MAX, &random);
        assert_eq!(decode_id(&late, IdPrefix::Run).0, MAX_ID_TIME_UNIX_MS);
    }

    #[test]
    fn id_has_prefix_refuses_other_kinds_and_bad_bodies() {
        let id = format_id(IdPrefix::Run, NOON_UNIX_MS, &[7; ID_RANDOM_BYTES]);
        assert!(id_has_prefix(&id, IdPrefix::Run));
        // "ro" is a prefix of "run" as text; the separator keeps them apart.
        assert!(!id_has_prefix(&id, IdPrefix::Rollout));
        assert!(!id_has_prefix(&id.replacen("run_", "run-", 1), IdPrefix::Run));
        assert!(!id_has_prefix(&id.to_lowercase(), IdPrefix::Run));
        assert!(!id_has_prefix(&id[..id.len() - 1], IdPrefix::Run));
        assert!(!id_has_prefix(&format!("{id}0"), IdPrefix::Run));
        assert!(!id_has_prefix("run_../../etc/passwd", IdPrefix::Run));
        assert!(!id_has_prefix("", IdPrefix::Run));
        // The first character holds 2 padding bits and 3 time bits, so it is at most 7.
        assert!(!id_has_prefix("run_80000000000000000000000000", IdPrefix::Run));
    }

    #[test]
    fn node_ids_encode_the_public_key() {
        let key = [0x42; 32];
        let node_id = node_id_from_public_key(&key);
        let body = node_id.strip_prefix("node_").unwrap();
        assert_eq!(body.len(), 52);
        assert_eq!(decode_base32(body, 32).unwrap(), key.to_vec());
        assert_ne!(node_id_from_public_key(&[0x43; 32]), node_id);
    }
}
