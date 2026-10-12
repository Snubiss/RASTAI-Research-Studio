//! `owner.token` (contracts 8.1, ADR-0034 decision 8): created 0600 on the first start, checked on every start, and
//! never rotated or rewritten by the daemon. Any mode other than 0600 stops the start with `RRS-C1008` and the fix
//! `chmod 600 <path>`: a mode group or others can read may mean someone already read the token, and one they can
//! write may mean someone replaced it with a token they know; quietly replacing it would hide that and break every
//! script that holds it. A token that can't be read or created, or whose text isn't 64 lowercase hex characters,
//! stops it with `RRS-C1010`. Both happen before the databases open and before any listener is bound, so a refused
//! start leaves nothing half started.
//!
//! The daemon keeps only the token's hash: the gate compares a bearer token's hash with it (http/gate.rs).
//!
//! The daemon's identity key (`daemon.key`) also belongs here, through `rastai_node::identity`, the one Ed25519
//! wrapper; it arrives with that crate (M1-42) and the embedded node's handshake (M1-47).

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use rastai_core::paths::{DataPaths, owner_token_path};
use rastai_core::random::random_bytes;
use rastai_formats::hashing::hash_bytes;

use crate::error::ServerError;

/// The token is 32 random bytes written as 64 lowercase hex characters, like every other secret the daemon hands
/// out (login codes, session keys), so it is plain text a script can paste into a header.
const OWNER_TOKEN_BYTES: usize = 32;
/// The token's text: two hex characters per byte.
const OWNER_TOKEN_TEXT_LENGTH: usize = OWNER_TOKEN_BYTES * 2;
/// The one byte the file may hold after the token (contracts 8.1): a newline a hand edit may leave.
const TRAILING_NEWLINE: u8 = b'\n';
/// The longest valid file is the token and its newline. One byte more is read, so a longer file is told apart
/// without reading all of it.
const OWNER_TOKEN_READ_LIMIT: u64 = OWNER_TOKEN_TEXT_LENGTH as u64 + 2;
/// The suffix of the temporary file a new token is written to before it is renamed into place (contracts 11.1).
const TEMPORARY_SUFFIX: &str = ".tmp";
/// Only the owner may read or write the token, and this is the only mode a start accepts.
#[cfg(unix)]
const OWNER_TOKEN_MODE: u32 = 0o600;
/// The `auth` folder holds only secrets, so only the owner may list or enter it.
#[cfg(unix)]
const AUTH_FOLDER_MODE: u32 = 0o700;
/// The permission bits of a mode, with setuid, setgid and sticky; the file-type bits above them are left out.
#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o7777;

/// Checks the owner token, or creates it on the first start, and returns the hash of its text. Never rewrites an
/// existing file. Reading needs only read access: the `auth` folder must be writable only when the token is created.
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
fn read_existing_token(path: &Path, file: File) -> Result<String, ServerError> {
    let metadata = file.metadata().map_err(|error| unusable(path, error))?;
    if !metadata.is_file() {
        return Err(unusable(path, "it is not a regular file"));
    }
    check_token_mode(path, &metadata)?;
    let mut text = Vec::new();
    file.take(OWNER_TOKEN_READ_LIMIT)
        .read_to_end(&mut text)
        .map_err(|error| unusable(path, error))?;
    // Anything else (empty, truncated, hand-edited such as "3f") would be a weak owner credential another local
    // user could guess over the loopback listener, so it is refused, never accepted or rewritten.
    let Some(token) = owner_token_from_text(&text) else {
        return Err(unusable(path, "it doesn't hold a token of 64 lowercase hex characters"));
    };
    Ok(hash_bytes(token))
}

/// The token in a file's text: exactly 64 lowercase hex characters, with at most one newline after them.
fn owner_token_from_text(text: &[u8]) -> Option<&[u8]> {
    let token = match text.split_last() {
        Some((&TRAILING_NEWLINE, before_newline)) => before_newline,
        _ => text,
    };
    if token.len() != OWNER_TOKEN_TEXT_LENGTH {
        return None;
    }
    if !token.iter().all(|&byte| is_lowercase_hex_digit(byte)) {
        return None;
    }
    Some(token)
}

fn is_lowercase_hex_digit(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Exactly 0600: a token group or others can read may have been read, and one they can write may have been
/// replaced; a mode the owner can't read or write (0400, 0700, setuid) isn't the file this daemon writes either.
#[cfg(unix)]
fn check_token_mode(path: &Path, metadata: &fs::Metadata) -> Result<(), ServerError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & PERMISSION_BITS;
    if mode != OWNER_TOKEN_MODE {
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

/// Writes a new token atomically (contracts 8.1, 11.1): the text goes to a temporary file in the same folder,
/// created 0600 when it is opened, so it is never readable by others, not even briefly; it is synced, renamed over
/// the token's path and the folder synced, so a crash leaves no token or a whole one, never a partial one. The
/// daemon lock is held (daemon.rs takes it first), so no other daemon can write the token between the open in
/// `ensure_owner_token` and the rename.
fn create_owner_token(path: &Path) -> Result<String, ServerError> {
    create_auth_folder(path)?;
    let token = new_owner_token()?;
    let temporary_path = temporary_token_path(path);
    remove_leftover_temporary_file(path, &temporary_path)?;
    let mut file = open_new_private_file(path, &temporary_path)?;
    let written = write_and_sync(&mut file, token.as_bytes());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary_path);
        return Err(unusable(path, error));
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(unusable(path, error));
    }
    sync_folder(path)?;
    Ok(hash_bytes(token.as_bytes()))
}

fn new_owner_token() -> Result<String, ServerError> {
    let bytes = random_bytes(OWNER_TOKEN_BYTES)?;
    Ok(lowercase_hex(&bytes))
}

/// `owner.token.tmp`, beside the token.
fn temporary_token_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(TEMPORARY_SUFFIX);
    path.with_file_name(name)
}

/// A start that crashed while writing the token may have left its temporary file. It never held a token anyone
/// was given, so it is removed; without that, `create_new` below would refuse every later start.
fn remove_leftover_temporary_file(path: &Path, temporary_path: &Path) -> Result<(), ServerError> {
    match fs::remove_file(temporary_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(unusable(path, error)),
    }
}

fn write_and_sync(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
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

/// `create_new` refuses a file or a symbolic link already at the path, so nothing planted there is written
/// through. The mode is given to `open`, so the file exists only as 0600 (less any umask bits). Setting it again on
/// the empty file, before any byte is written, undoes a umask that also took the owner's bits away, so the token
/// written is the 0600 file the next start accepts.
#[cfg(unix)]
fn open_new_private_file(token_path: &Path, temporary_path: &Path) -> Result<File, ServerError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(OWNER_TOKEN_MODE)
        .open(temporary_path)
        .map_err(|error| unusable(token_path, error))?;
    if let Err(error) = file.set_permissions(fs::Permissions::from_mode(OWNER_TOKEN_MODE)) {
        drop(file);
        let _ = fs::remove_file(temporary_path);
        return Err(unusable(token_path, error));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_new_private_file(token_path: &Path, temporary_path: &Path) -> Result<File, ServerError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|error| unusable(token_path, error))
}

/// The rename is durable only once the folder's entry is on disk.
#[cfg(unix)]
fn sync_folder(token_path: &Path) -> Result<(), ServerError> {
    let Some(folder) = token_path.parent() else {
        return Ok(());
    };
    File::open(folder)
        .and_then(|opened| opened.sync_all())
        .map_err(|error| unusable(token_path, error))
}

/// Windows can't open a folder as a file; NTFS journals the rename itself.
#[cfg(not(unix))]
fn sync_folder(_token_path: &Path) -> Result<(), ServerError> {
    Ok(())
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

    const VALID_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn only_64_lowercase_hex_characters_and_one_newline_are_a_token() {
        assert_eq!(
            owner_token_from_text(VALID_TOKEN.as_bytes()),
            Some(VALID_TOKEN.as_bytes())
        );
        let with_newline = format!("{VALID_TOKEN}\n");
        assert_eq!(
            owner_token_from_text(with_newline.as_bytes()),
            Some(VALID_TOKEN.as_bytes())
        );

        let uppercase = VALID_TOKEN.to_uppercase();
        let two_newlines = format!("{VALID_TOKEN}\n\n");
        let carriage_return = format!("{VALID_TOKEN}\r\n");
        let leading_space = format!(" {VALID_TOKEN}");
        let too_long = format!("{VALID_TOKEN}0");
        let not_hex = VALID_TOKEN.replace('a', "g");
        let malformed: [&[u8]; 11] = [
            b"",
            b"\n",
            b"3f",
            b"password",
            &VALID_TOKEN.as_bytes()[..63],
            too_long.as_bytes(),
            uppercase.as_bytes(),
            two_newlines.as_bytes(),
            carriage_return.as_bytes(),
            leading_space.as_bytes(),
            not_hex.as_bytes(),
        ];
        for text in malformed {
            assert_eq!(owner_token_from_text(text), None, "{:?}", String::from_utf8_lossy(text));
        }
    }

    #[cfg(unix)]
    mod on_unix {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        /// A fresh `auth` folder for one test, under the system temporary folder.
        fn test_token_path(test_name: &str) -> PathBuf {
            let root = std::env::temp_dir().join(format!("rsv-auth-{}-{test_name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            root.join("auth").join("owner.token")
        }

        fn remove_test_root(token_path: &Path) {
            let root = token_path.parent().unwrap().parent().unwrap();
            fs::remove_dir_all(root).unwrap();
        }

        fn mode_of(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
        }

        #[test]
        fn the_temporary_file_is_private_from_the_moment_it_exists() {
            let token_path = test_token_path("tempmode");
            create_auth_folder(&token_path).unwrap();
            let temporary_path = temporary_token_path(&token_path);
            let file = open_new_private_file(&token_path, &temporary_path).unwrap();
            // Nothing written yet: the file already has its final mode, so no byte is ever readable by others.
            assert_eq!(fs::metadata(&temporary_path).unwrap().len(), 0);
            assert_eq!(mode_of(&temporary_path), OWNER_TOKEN_MODE);
            drop(file);
            remove_test_root(&token_path);
        }

        #[test]
        fn the_first_write_is_atomic_private_and_leaves_no_temporary_file() {
            let token_path = test_token_path("firstwrite");
            let hash = create_owner_token(&token_path).unwrap();
            assert_eq!(mode_of(&token_path), OWNER_TOKEN_MODE);
            assert_eq!(mode_of(token_path.parent().unwrap()), AUTH_FOLDER_MODE);
            assert!(!temporary_token_path(&token_path).exists());
            let entries = fs::read_dir(token_path.parent().unwrap()).unwrap().count();
            assert_eq!(entries, 1, "only owner.token is left in the folder");

            let text = fs::read(&token_path).unwrap();
            assert_eq!(owner_token_from_text(&text), Some(text.as_slice()));
            let file = File::open(&token_path).unwrap();
            assert_eq!(read_existing_token(&token_path, file).unwrap(), hash);
            remove_test_root(&token_path);
        }

        #[test]
        fn a_temporary_file_left_by_a_crash_is_replaced_not_reused() {
            let token_path = test_token_path("leftover");
            create_auth_folder(&token_path).unwrap();
            let temporary_path = temporary_token_path(&token_path);
            fs::write(&temporary_path, "3f").unwrap();
            fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o644)).unwrap();

            create_owner_token(&token_path).unwrap();
            assert_eq!(mode_of(&token_path), OWNER_TOKEN_MODE);
            assert!(!temporary_path.exists());
            let text = fs::read(&token_path).unwrap();
            assert_eq!(text.len(), OWNER_TOKEN_TEXT_LENGTH);
            remove_test_root(&token_path);
        }

        #[test]
        fn any_mode_but_0600_is_rrs_c1008() {
            let token_path = test_token_path("modes");
            create_owner_token(&token_path).unwrap();
            for mode in [0o620, 0o602, 0o640, 0o604, 0o644, 0o400, 0o700, 0o4600] {
                fs::set_permissions(&token_path, fs::Permissions::from_mode(mode)).unwrap();
                let file = File::open(&token_path).unwrap();
                let refused = read_existing_token(&token_path, file).err().unwrap();
                assert_eq!(refused.code(), "RRS-C1008", "mode {mode:o}");
            }
            remove_test_root(&token_path);
        }
    }
}
