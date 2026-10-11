# ADR-0033: `unsafe` is allowed for one `statvfs` call, in `rastai-store/src/disk_space.rs` only

**Status:** Accepted, 2026-10-10

## Context

- Free space is checked before downloads, checkpoints and training (ADR-0003 decision 7, `storage.free_space_margin_bytes`), and the node reports it in `NodeTelemetry.disk_free_bytes`.
- Rust's standard library has no free-space API. On Linux and macOS the answer comes from `statvfs(3)`, a C call. Calling it from Rust needs `libc` and an `unsafe` block.
- `unsafe_code` is denied in the whole workspace (ADR-0023, contracts §12.2), and the spike's `mmap` was dropped for that reason.
- The drafts disagreed: the store used `statvfs` and the node ran `df` (plan §10 row 15). Two ways to get one number drift.
- `libc` is already in the dependency tree under tokio, so allowing it directly adds no new code to the build, only a direct import.

## Decision

1. **`libc` is on the §27 allowlist** (Foundation: the OS), owned by `rastai-store` and imported only in `crates/rastai-store/src/disk_space.rs`. `deny.toml` `wrappers` pins it to `rastai-store`; `check-imports` pins it to that file.
2. **That file holds the only `unsafe` in the workspace:** one `unsafe` block around one `libc::statvfs` call, inside one small function. The function carries `#[allow(unsafe_code)]`; the rest of the crate and workspace keep `unsafe_code` denied.
3. **One free-space path for everyone.** `rastai_store::free_bytes(path)` and `check_free_space` serve the daemon, the node (downloads, telemetry, hardware detection) and every free-space check in contracts §8.1. Free bytes are `f_bavail × f_frsize`: the space an unprivileged user can actually write.
4. **A failed call never guesses.** `free_bytes` returns a typed `StoreError`, and callers treat "unknown" like "not enough": the check fails with `RRS-S3001` instead of being skipped.
5. **Tier 2:** the body is `#[cfg(unix)]`, so macOS uses the same call. Windows gets a body that returns a typed "not supported yet" error until tier 2 work starts (ADR-0014).
6. **The list of `unsafe` files is closed.** `tools/checks/check-unsafe` holds the allowed files (this one) and fails CI on `unsafe` anywhere else. Another file needs a new ADR that names it.
7. **Fallback if this ADR is ever withdrawn:** `free_bytes` runs the OS tool `df -B1 --output=avail <path>` (already on §27, Linux only) behind the same signature. No caller changes.

```rust
// crates/rastai-store/src/disk_space.rs
// The only `unsafe` in the workspace (ADR-0033): std has no free-space API.

#[cfg(unix)]
#[allow(unsafe_code)]
pub fn free_bytes(path: &Path) -> Result<u64, StoreError> {
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| StoreError::BadPath(path.to_path_buf()))?;
    // SAFETY: c_path is NUL-terminated and outlives the call; statvfs only writes into `stats`.
    let (result, stats) = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        let result = libc::statvfs(c_path.as_ptr(), &mut stats);
        (result, stats)
    };
    if result != 0 {
        return Err(StoreError::FreeSpaceUnknown(path.to_path_buf()));
    }
    Ok(stats.f_bavail as u64 * stats.f_frsize as u64)
}
```

The error variant names are illustrations; the control-plane layout owns `StoreError`'s variants.

**Checked by:**
- `check-unsafe` in `just check`: a test file with `unsafe` outside `disk_space.rs` fails it;
- `check-imports` and `deny.toml`: `libc` imported anywhere else fails;
- a unit test comparing `free_bytes` on a temp directory with `df -B1 --output=avail` on the CI runner (within one block);
- a unit test that a missing path returns the typed error, and that `check_free_space` then fails with `RRS-S3001`.

## Consequences

**Good**
- One free-space number, one function, used by the daemon and the node alike.
- No subprocess per check, and no text parsing of `df` output.
- The `unsafe` surface is three lines a reviewer can read in a minute, with the reason above it.

**Costs**
- The workspace is no longer `unsafe`-free. The exception is narrow and listed, but it's real.
- `libc` becomes a direct dependency to track (it already was a transitive one).
- Windows needs its own body later (`GetDiskFreeSpaceExW`), which will need its own ADR entry or this one's amendment.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Run `df` for every check | A process per check, Linux-only flags, and text parsing. Kept as the documented fallback. |
| A crate (`fs2`, `fs4`, `sysinfo`) | An external dependency outside the five categories for one call, with its own `unsafe` inside anyway. |
| Skip free-space checks | ADR-0003 requires them: a full disk mid-checkpoint costs hours of training. |
| Read `/proc/mounts` and guess | Doesn't give free space. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 19, [§19](../spec/rastai_spec_v5.md#19-storage-and-durability-new), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new), [§30](../spec/rastai_spec_v5.md#30-adrs-to-write-now)
- Contracts: [§2.5 libc row](../architecture/contracts.md#25-external-libraries-and-their-one-wrapper-file), [§12.2 Rust rules](../architecture/contracts.md#122-rust), [§16 row 15](../architecture/contracts.md#16-plan-10-conflicts-the-answers-in-v11)
- Plan: M1-02 (writes this ADR and the §27 row), M1-25 (`disk_space.rs`)
- Related: [ADR-0003](0003-storage-sqlite-vault-blobs.md), [ADR-0012](0012-build-in-house-policy.md), [ADR-0014](0014-platform-tiers.md), [ADR-0023](0023-owner-code-style.md)
