//! `owner.token` (contracts 8.1, ADR-0034 decision 8): created 0600 on the first start, checked on every start, and
//! never rotated or rewritten by the daemon. A token that group or others can read stops the start with `RRS-C1008`
//! and the fix `chmod 600 <path>`, because a loose mode may mean someone already read it; quietly replacing it would
//! hide that and break every script that holds it. A token that can't be read or created stops it with `RRS-C1010`.
//! Both happen before the databases open and before any listener is bound, so a refused start leaves nothing
//! half started.
//!
//! The daemon keeps only the token's hash: the gate compares a bearer token's hash with it (http/gate.rs).
//!
//! The daemon's identity key (`daemon.key`) also belongs here, through `rastai_node::identity`, the one Ed25519
//! wrapper; it arrives with that crate (M1-42) and the embedded node's handshake (M1-47).

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use rastai_core::paths::{DataPaths, owner_token_path};
use rastai_core::random::random_bytes;
use rastai_formats::hashing::hash_bytes;

use crate::error::ServerError;

/// The token is 32 random bytes written as 64 lowercase hex characters, like every other secret the daemon hands
/// out (login codes, session keys), so it is plain text a script can paste into a header.
const OWNER_TOKEN_BYTES: usize = 32;
/// Only the owner may read or write the token.
#[cfg(unix)]
const OWNER_TOKEN_MODE: u32 = 0o600;
/// The `auth` folder holds only secrets, so only the owner may list or enter it.
#[cfg(unix)]
const AUTH_FOLDER_MODE: u32 = 0o700;
/// Group and others bits: any of them set on the token refuses the start.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;
/// The permission bits of a mode; the file-type bits above them are left out of the error.
#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o777;

/// Checks the owner token, or creates it on the first start, and returns the hash of its text. Never rewrites an
/// existing file.
pub fn ensure_owner_token(paths: &DataPaths) -> Result<String, ServerError> {
    let path = owner_token_path(paths);
    match File::open(&path) {
        Ok(file) => read_existing_token(&path, file),
        Err(error) if error.kind() == ErrorKind::NotFound => create_owner_token(&path),
        Err(error) => Err(unusable(&path, error)),
    }
}

/// Checks the open file's own metadata, not the path's, so the mode checked is the mode of the bytes read even if
/// the path is swapped in between. A symbolic link is followed: what matters is who can read the token itself.
fn read_existing_token(path: &Path, mut file: File) -> Result<String, ServerError> {
    let metadata = file.metadata().map_err(|error| unusable(path, error))?;
    if !metadata.is_file() {
        return Err(unusable(path, "it is not a regular file"));
    }
    check_token_mode(path, &metadata)?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|error| unusable(path, error))?;
    // A hand edit may leave a newline at the end; the token is the text without it.
    let token = text.trim();
    if token.is_empty() {
        return Err(unusable(path, "it is empty"));
    }
    Ok(hash_bytes(token.as_bytes()))
}

#[cfg(unix)]
fn check_token_mode(path: &Path, metadata: &fs::Metadata) -> Result<(), ServerError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & PERMISSION_BITS;
    if mode & GROUP_AND_OTHER_BITS != 0 {
        return Err(ServerError::OwnerTokenTooOpen {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

/// Windows has no mode bits; its ACL check arrives with the Windows build (spec §28).
#[cfg(not(unix))]
fn check_token_mode(_path: &Path, _metadata: &fs::Metadata) -> Result<(), ServerError> {
    Ok(())
}

/// Writes a new token. `create_new` refuses a file that appeared since the open above, so a token is never
/// overwritten, and the mode is set at creation, so the token is never readable by others, not even briefly.
fn create_owner_token(path: &Path) -> Result<String, ServerError> {
    create_auth_folder(path)?;
    let token = new_owner_token()?;
    let mut file = open_new_private_file(path)?;
    file.write_all(token.as_bytes())
        .map_err(|error| unusable(path, error))?;
    file.sync_all().map_err(|error| unusable(path, error))?;
    Ok(hash_bytes(token.as_bytes()))
}

fn new_owner_token() -> Result<String, ServerError> {
    let bytes = random_bytes(OWNER_TOKEN_BYTES)?;
    Ok(lowercase_hex(&bytes))
}

#[cfg(unix)]
fn create_auth_folder(token_path: &Path) -> Result<(), ServerError> {
    use std::os::unix::fs::DirBuilderExt;

    let Some(folder) = token_path.parent() else {
        return Ok(());
    };
    fs::DirBuilder::new()
        .recursive(true)
        .mode(AUTH_FOLDER_MODE)
        .create(folder)
        .map_err(|error| unusable(token_path, error))
}

#[cfg(not(unix))]
fn create_auth_folder(token_path: &Path) -> Result<(), ServerError> {
    let Some(folder) = token_path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(folder).map_err(|error| unusable(token_path, error))
}

#[cfg(unix)]
fn open_new_private_file(path: &Path) -> Result<File, ServerError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(OWNER_TOKEN_MODE)
        .open(path)
        .map_err(|error| unusable(path, error))
}

#[cfg(not(unix))]
fn open_new_private_file(path: &Path) -> Result<File, ServerError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| unusable(path, error))
}

/// Bytes as lowercase hex, two characters each: the text form of every secret the daemon hands out.
pub fn lowercase_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn unusable(path: &Path, reason: impl std::fmt::Display) -> ServerError {
    ServerError::OwnerTokenUnusable {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_two_lowercase_digits_per_byte() {
        assert_eq!(lowercase_hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(lowercase_hex(&[]), "");
    }

    #[test]
    fn a_new_token_is_64_hex_characters() {
        let token = new_owner_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(
            token
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
    }
}
