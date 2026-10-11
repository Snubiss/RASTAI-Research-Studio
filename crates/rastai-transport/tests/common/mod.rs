//! The test folder every socket test starts from, shared by tests/local_socket.rs, tests/grpc_local.rs and
//! tests/http_client.rs. Unix only, like the tests that use it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The socket's folder inside a test folder; `bind_local_socket` creates it 0700.
pub const SOCKET_FOLDER: &str = "run";

/// Numbers the folders of one test binary, so tests running at the same time never share one.
static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty folder for one test. `prefix` names the test file; the process id keeps two test binaries apart.
/// The name stays short so a socket path inside it fits the 107 bytes a Unix socket address holds.
#[allow(clippy::unwrap_used)]
pub fn new_test_folder(prefix: &str) -> PathBuf {
    let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
    let test_folder = std::env::temp_dir().join(format!("{prefix}-{}-{number}", std::process::id()));
    let _ = fs::remove_dir_all(&test_folder);
    fs::create_dir_all(&test_folder).unwrap();
    test_folder
}

/// The socket path inside a test folder: `<test folder>/run/<socket_file>`.
pub fn socket_path_in(test_folder: &Path, socket_file: &str) -> PathBuf {
    test_folder.join(SOCKET_FOLDER).join(socket_file)
}
