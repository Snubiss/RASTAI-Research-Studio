//! Free disk space (ADR-0033, layout rust-control-plane.md 6.4 and 6.7, flows/platform.md 8.1): `free_bytes` agrees
//! with `df`, a missing path gives the typed error, and `check_free_space` fails with `RRS-S3001` below the margin
//! and when the space is unknown.

#![allow(clippy::unwrap_used)]

use rastai_store::disk_space::{check_free_space, free_bytes};
use rastai_store::error::StoreError;

mod common;
use common::{remove_test_folder, test_paths};

// Far more than any test machine's disk, so a check that needs it always fails.
const MORE_THAN_ANY_DISK: u64 = 1 << 62;

fn test_folder(test_name: &str) -> (rastai_core::paths::DataPaths, std::path::PathBuf) {
    let paths = test_paths(test_name);
    std::fs::create_dir_all(&paths.data).unwrap();
    let folder = paths.data.clone();
    (paths, folder)
}

#[test]
fn there_is_room_for_nothing_and_not_for_more_than_a_disk() {
    let (paths, folder) = test_folder("disk-space-room");
    let free = free_bytes(&folder).unwrap();
    assert!(free > 0);

    check_free_space(&folder, 0, 0).unwrap();
    let error = check_free_space(&folder, MORE_THAN_ANY_DISK, 0).unwrap_err();

    assert_eq!(error.code(), "RRS-S3001");
    let StoreError::NotEnoughFreeSpace {
        needed_bytes,
        free_bytes,
        ..
    } = error
    else {
        panic!("expected not enough free space");
    };
    assert_eq!(needed_bytes, MORE_THAN_ANY_DISK);
    assert!(free_bytes > 0);
    remove_test_folder(&paths);
}

#[test]
fn the_margin_counts_against_the_free_space() {
    let (paths, folder) = test_folder("disk-space-margin");
    let free = free_bytes(&folder).unwrap();

    // One byte needed fits on the disk, but not once the margin keeps everything that is free.
    let error = check_free_space(&folder, 1, free).unwrap_err();
    assert_eq!(error.code(), "RRS-S3001");
    let StoreError::NotEnoughFreeSpace { margin_bytes, .. } = error else {
        panic!("expected not enough free space");
    };
    assert_eq!(margin_bytes, free);
    // A margin and a need that overflow together never fit.
    assert_eq!(
        check_free_space(&folder, u64::MAX, u64::MAX).unwrap_err().code(),
        "RRS-S3001"
    );
    remove_test_folder(&paths);
}

#[test]
fn a_missing_path_is_unknown_and_unknown_is_not_enough() {
    let (paths, folder) = test_folder("disk-space-missing");
    let missing = folder.join("no-such-folder");

    let error = free_bytes(&missing).unwrap_err();
    assert!(matches!(error, StoreError::FreeSpaceUnknown { .. }), "{error:?}");

    // ADR-0033 decision 4: the check fails instead of being skipped.
    let error = check_free_space(&missing, 0, 0).unwrap_err();
    assert_eq!(error.code(), "RRS-S3001");
    remove_test_folder(&paths);
}

/// `df -B1 --output=avail` is the same figure (f_bavail × f_frsize). The disk is shared with every other test
/// running now, so the free space can move between the two calls: `df` must fall within one block of the range
/// `free_bytes` saw just before and just after it. A few tries ride out a burst of writes elsewhere.
#[cfg(target_os = "linux")]
#[test]
fn free_bytes_agrees_with_df() {
    const TRIES: usize = 5;
    const ONE_BLOCK: u64 = 4096;
    let (paths, folder) = test_folder("disk-space-df");
    let mut last_seen = (0, 0, 0);
    let mut agreed = false;
    for _ in 0..TRIES {
        let before = free_bytes(&folder).unwrap();
        let reported = df_available_bytes(&folder);
        let after = free_bytes(&folder).unwrap();
        let low = before.min(after).saturating_sub(ONE_BLOCK);
        let high = before.max(after) + ONE_BLOCK;
        last_seen = (before, reported, after);
        if (low..=high).contains(&reported) {
            agreed = true;
            break;
        }
    }
    assert!(agreed, "free_bytes before, df, free_bytes after: {last_seen:?}");
    remove_test_folder(&paths);
}

#[cfg(target_os = "linux")]
fn df_available_bytes(folder: &std::path::Path) -> u64 {
    let output = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(folder)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    // The first line is the column title, "Avail".
    text.lines().nth(1).unwrap().trim().parse().unwrap()
}
