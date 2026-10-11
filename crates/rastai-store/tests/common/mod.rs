//! What the integration tests share: a fresh set of data folders for each test, removed again at the test's end.
//! A test that fails stops before the removal, so its folder stays behind to be looked at.

use rastai_core::paths::DataPaths;

/// Empty data folders for one test, all under one parent named after the test. Each test binary is its own
/// process, so the process id keeps two binaries running at once apart.
pub fn test_paths(test_name: &str) -> DataPaths {
    let root = std::env::temp_dir().join(format!("rastai-store-{}-{test_name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    DataPaths {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
        state: root.join("state"),
        runtime: root.join("runtime"),
    }
}

/// Removes the parent folder `test_paths` made. Call it after the store is closed.
pub fn remove_test_folder(paths: &DataPaths) {
    if let Some(root) = paths.data.parent() {
        let _ = std::fs::remove_dir_all(root);
    }
}
