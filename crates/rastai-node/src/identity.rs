//! Ed25519 identities: load or create a key file, sign and verify challenges (layout: rust-node-gateway-proto.md
//! section 7.2). The one file in the workspace that imports ed25519-dalek (contracts 2.5): the node uses it for
//! node.key and the handshake, and rastai-server for daemon.key, through these functions only.
//!
//! A key file holds the 32-byte Ed25519 secret seed and nothing else, mode 0600. A new one is written to a
//! temporary file first and then hard-linked into place, which fails when a key already exists, so two processes
//! starting at once can never replace each other's key: the loser reads the winner's. A key that can't be read, has
//! the wrong size, or is open to group or others is refused with RRS-N0006. It is never replaced, because a new key
//! is a new node id, which would silently break every pairing.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rastai_core::crockford::{decode_base32, encode_base32};
use rastai_core::random::random_bytes;

use crate::error::NodeError;

/// Ed25519 secret seeds, public keys and signatures have fixed sizes.
const SECRET_KEY_BYTES: usize = 32;
const PUBLIC_KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;
/// Key files are the owner's alone (contracts 8.1).
#[cfg(unix)]
const KEY_FILE_MODE: u32 = 0o600;
/// The identity folder is created private too, since it holds nothing but keys.
#[cfg(unix)]
const KEY_FOLDER_MODE: u32 = 0o700;
/// Permission bits a key file must not have: any access for group or others.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;
/// The temporary file beside the key while a new one is written.
const NEW_KEY_SUFFIX: &str = "new";
/// The daemon's id prefix: `daemon_<base32 of its public key>` (contracts 8.6). Node ids come from rastai-core.
const DAEMON_ID_PREFIX: &str = "daemon_";

/// One Ed25519 identity. `public_id` is the base32 of the public key; `node_` or `daemon_` before it is the id.
pub struct NodeIdentity {
    pub signing_key: SigningKey,
    pub public_id: String,
}

/// The identity in `key_path`, made and saved first when the file doesn't exist yet.
pub fn load_or_create_identity(key_path: &Path) -> Result<NodeIdentity, NodeError> {
    match read_key_file(key_path)? {
        Some(secret) => Ok(identity_from_secret(&secret)),
        None => create_identity(key_path),
    }
}

/// The 32-byte public key of an identity.
pub fn public_key_bytes(identity: &NodeIdentity) -> [u8; PUBLIC_KEY_BYTES] {
    identity.signing_key.verifying_key().to_bytes()
}

/// `daemon_<base32 of the public key>`, the daemon's id and the archive's source id (contracts 8.6).
pub fn daemon_id_from_public_key(public_key: &[u8; PUBLIC_KEY_BYTES]) -> String {
    format!("{DAEMON_ID_PREFIX}{}", encode_base32(public_key))
}

/// The public key an id names (`daemon_...` or `node_...`, given its prefix with the `_`), or None when the id
/// doesn't have that prefix or its body isn't a canonical 32-byte base32 text.
pub fn public_key_from_id(id: &str, prefix: &str) -> Option<[u8; PUBLIC_KEY_BYTES]> {
    let body = id.strip_prefix(prefix)?;
    let bytes = decode_base32(body, PUBLIC_KEY_BYTES)?;
    bytes.try_into().ok()
}

/// The identity's Ed25519 signature over `challenge`.
pub fn sign_challenge(identity: &NodeIdentity, challenge: &[u8]) -> Vec<u8> {
    identity.signing_key.sign(challenge).to_bytes().to_vec()
}

/// Ok when `signature` is the signature of `public_key` over `challenge`. Strict verification, so a signature
/// with a malleated form or a weak public key is refused too. Any refusal is `SignatureRefused` (RRS-N0002).
pub fn verify_challenge(public_key: &[u8], challenge: &[u8], signature: &[u8]) -> Result<(), NodeError> {
    let key_bytes: [u8; PUBLIC_KEY_BYTES] = public_key
        .try_into()
        .map_err(|_| signature_refused(format!("a public key has {} bytes, not 32", public_key.len())))?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|error| signature_refused(format!("the public key isn't a curve point: {error}")))?;
    let signature_bytes: [u8; SIGNATURE_BYTES] = signature
        .try_into()
        .map_err(|_| signature_refused(format!("a signature has {} bytes, not 64", signature.len())))?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify_strict(challenge, &signature)
        .map_err(|error| signature_refused(format!("the signature doesn't verify: {error}")))
}

fn identity_from_secret(secret: &[u8; SECRET_KEY_BYTES]) -> NodeIdentity {
    let signing_key = SigningKey::from_bytes(secret);
    let public_id = encode_base32(&signing_key.verifying_key().to_bytes());
    NodeIdentity { signing_key, public_id }
}

/// The secret seed in the key file, or None when there is no file. A file that exists but can't be used is an
/// error, never "no file".
fn read_key_file(key_path: &Path) -> Result<Option<[u8; SECRET_KEY_BYTES]>, NodeError> {
    let metadata = match fs::symlink_metadata(key_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unusable(key_path, format!("can't be read: {error}"))),
    };
    if !metadata.file_type().is_file() {
        return Err(unusable(
            key_path,
            "it isn't a regular file (a symbolic link or a folder)",
        ));
    }
    check_private_mode(key_path, &metadata)?;
    let bytes = fs::read(key_path).map_err(|error| unusable(key_path, format!("can't be read: {error}")))?;
    let secret: [u8; SECRET_KEY_BYTES] = bytes.as_slice().try_into().map_err(|_| {
        unusable(
            key_path,
            format!("it holds {} bytes, not the 32 of an Ed25519 key", bytes.len()),
        )
    })?;
    Ok(Some(secret))
}

/// A key file anyone but its owner can open is refused: whoever can read it can act as this node.
#[cfg(unix)]
fn check_private_mode(key_path: &Path, metadata: &fs::Metadata) -> Result<(), NodeError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o777;
    if mode & GROUP_AND_OTHER_BITS != 0 {
        return Err(unusable(
            key_path,
            format!("its mode is {mode:04o}; only its owner may read it ({KEY_FILE_MODE:04o})"),
        ));
    }
    Ok(())
}

/// Windows file access is set by ACLs, which the Windows build checks when it lands (tier 2).
#[cfg(not(unix))]
fn check_private_mode(_key_path: &Path, _metadata: &fs::Metadata) -> Result<(), NodeError> {
    Ok(())
}

/// Makes a key, writes it beside `key_path` and links it into place. If another process saved a key first, that
/// key is the identity and ours is dropped.
fn create_identity(key_path: &Path) -> Result<NodeIdentity, NodeError> {
    create_key_folder(key_path)?;
    let random = random_bytes(SECRET_KEY_BYTES)?;
    let secret: [u8; SECRET_KEY_BYTES] = random
        .as_slice()
        .try_into()
        .map_err(|_| unusable(key_path, "the random source gave too few bytes"))?;
    let new_key_path = new_key_file_path(key_path);
    write_new_key_file(&new_key_path, &secret)?;
    let linked = fs::hard_link(&new_key_path, key_path);
    // The temporary name is only ever ours to remove; the key stays under its real name.
    let _ = fs::remove_file(&new_key_path);
    match linked {
        Ok(()) => Ok(identity_from_secret(&secret)),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => load_existing_identity(key_path),
        Err(error) => Err(unusable(key_path, format!("can't be saved: {error}"))),
    }
}

fn load_existing_identity(key_path: &Path) -> Result<NodeIdentity, NodeError> {
    match read_key_file(key_path)? {
        Some(secret) => Ok(identity_from_secret(&secret)),
        None => Err(unusable(key_path, "it disappeared while it was being created")),
    }
}

/// `<key>.new`. A file of that name left by a crash is replaced: nothing but this function writes it.
fn new_key_file_path(key_path: &Path) -> PathBuf {
    let mut file_name = key_path.file_name().unwrap_or_default().to_os_string();
    file_name.push(".");
    file_name.push(NEW_KEY_SUFFIX);
    key_path.with_file_name(file_name)
}

fn write_new_key_file(new_key_path: &Path, secret: &[u8; SECRET_KEY_BYTES]) -> Result<(), NodeError> {
    let _ = fs::remove_file(new_key_path);
    let mut file = open_private_file(new_key_path)?;
    file.write_all(secret)
        .and_then(|()| file.sync_all())
        .map_err(|error| unusable(new_key_path, format!("can't be written: {error}")))
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> Result<fs::File, NodeError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(KEY_FILE_MODE)
        .open(path)
        .map_err(|error| unusable(path, format!("can't be created: {error}")))
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> Result<fs::File, NodeError> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| unusable(path, format!("can't be created: {error}")))
}

/// Creates the key's folder 0700 when it is missing. An existing folder is left as it is.
fn create_key_folder(key_path: &Path) -> Result<(), NodeError> {
    let Some(folder) = key_path.parent() else {
        return Ok(());
    };
    if folder.as_os_str().is_empty() || folder.is_dir() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    set_private_folder_mode(&mut builder);
    builder
        .create(folder)
        .map_err(|error| unusable(key_path, format!("its folder can't be created: {error}")))
}

#[cfg(unix)]
fn set_private_folder_mode(builder: &mut fs::DirBuilder) {
    use std::os::unix::fs::DirBuilderExt;

    builder.mode(KEY_FOLDER_MODE);
}

#[cfg(not(unix))]
fn set_private_folder_mode(_builder: &mut fs::DirBuilder) {}

fn unusable(path: &Path, reason: impl std::fmt::Display) -> NodeError {
    NodeError::IdentityFileUnusable {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

fn signature_refused(reason: String) -> NodeError {
    NodeError::SignatureRefused { reason }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const CHALLENGE: &[u8] = b"rastai-node-handshake-v1 test challenge";
    static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

    fn new_test_folder() -> PathBuf {
        let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
        let folder = std::env::temp_dir().join(format!("rni-{}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[cfg(unix)]
    #[test]
    fn a_new_key_is_saved_private_and_loads_back_the_same() {
        use std::os::unix::fs::PermissionsExt;

        let folder = new_test_folder();
        let key_path = folder.join("identity").join("node.key");
        let created = load_or_create_identity(&key_path).unwrap();
        let loaded = load_or_create_identity(&key_path).unwrap();
        assert_eq!(created.public_id, loaded.public_id);
        assert_eq!(fs::read(&key_path).unwrap().len(), SECRET_KEY_BYTES);
        let mode = fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, KEY_FILE_MODE);
        let folder_mode = fs::metadata(key_path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(folder_mode, KEY_FOLDER_MODE);
        assert!(!new_key_file_path(&key_path).exists());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_signature_verifies_only_for_its_key_and_challenge() {
        let folder = new_test_folder();
        let first = load_or_create_identity(&folder.join("a.key")).unwrap();
        let second = load_or_create_identity(&folder.join("b.key")).unwrap();
        let signature = sign_challenge(&first, CHALLENGE);
        assert_eq!(signature.len(), SIGNATURE_BYTES);
        assert!(verify_challenge(&public_key_bytes(&first), CHALLENGE, &signature).is_ok());
        let wrong_key = verify_challenge(&public_key_bytes(&second), CHALLENGE, &signature).unwrap_err();
        assert_eq!(wrong_key.code(), "RRS-N0002");
        let wrong_challenge = verify_challenge(&public_key_bytes(&first), b"another challenge", &signature);
        assert!(wrong_challenge.is_err());
        let short_signature = verify_challenge(&public_key_bytes(&first), CHALLENGE, &signature[..10]);
        assert!(short_signature.is_err());
        let short_key = verify_challenge(&[1, 2, 3], CHALLENGE, &signature);
        assert!(short_key.is_err());
        let _ = fs::remove_dir_all(&folder);
    }

    // A damaged key is refused, never replaced: a new key would be a new node id.
    #[test]
    fn a_key_of_the_wrong_size_is_refused_and_left_alone() {
        let folder = new_test_folder();
        let key_path = folder.join("node.key");
        fs::write(&key_path, b"short").unwrap();
        set_owner_only(&key_path);
        let error = load_or_create_identity(&key_path).err().unwrap();
        assert_eq!(error.code(), "RRS-N0006");
        assert_eq!(fs::read(&key_path).unwrap(), b"short");
        let _ = fs::remove_dir_all(&folder);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let folder = new_test_folder();
        let key_path = folder.join("node.key");
        load_or_create_identity(&key_path).unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = load_or_create_identity(&key_path).err().unwrap();
        assert_eq!(error.code(), "RRS-N0006");
        assert!(format!("{error:?}").contains("0644"), "{error:?}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn ids_round_trip_to_their_public_key() {
        let folder = new_test_folder();
        let identity = load_or_create_identity(&folder.join("daemon.key")).unwrap();
        let public_key = public_key_bytes(&identity);
        let daemon_id = daemon_id_from_public_key(&public_key);
        assert_eq!(daemon_id, format!("daemon_{}", identity.public_id));
        assert_eq!(public_key_from_id(&daemon_id, DAEMON_ID_PREFIX), Some(public_key));
        assert_eq!(public_key_from_id(&daemon_id, "node_"), None);
        assert_eq!(public_key_from_id("daemon_TOOSHORT", DAEMON_ID_PREFIX), None);
        let _ = fs::remove_dir_all(&folder);
    }

    #[cfg(unix)]
    fn set_owner_only(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(KEY_FILE_MODE)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_owner_only(_path: &Path) {}
}
