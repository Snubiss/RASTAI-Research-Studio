"""check-unsafe on passing and failing fixture trees (ADR-0033)."""

ALLOWED_FILE = "crates/rastai-store/src/disk_space.rs"
ONE_STATVFS_CALL = """
#[allow(unsafe_code)]
fn statvfs_free_bytes(path: &CStr) -> Option<u64> {
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let status = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    None
}
"""


def test_passing_tree(write_tree, run_check):
    root = write_tree(
        {
            ALLOWED_FILE: ONE_STATVFS_CALL,
            "crates/rastai-formats/src/zip/reader.rs": "// no unsafe { } here\n"
            'const MESSAGE: &str = "unsafe path in the archive";\n',
            "spike/bakeoff/rust/src/mmap.rs": "fn map() { unsafe { } }\n",
            "target/debug/build/out.rs": "unsafe fn generated() {}\n",
        }
    )
    result = run_check("check-unsafe", "--root", str(root))
    assert result.returncode == 0, result.stdout


def test_unsafe_outside_the_exception_list_fails(write_tree, run_check):
    root = write_tree(
        {
            "crates/rastai-formats/src/safetensors/reader.rs": "fn read() {\n    let x = unsafe { read_raw() };\n}\n",
            "crates/rastai-core/src/ids.rs": "unsafe impl Send for Id {}\n",
            "crates/rastai-diag/src/lib.rs": "#[unsafe(no_mangle)]\npub fn entry() {}\n",
        }
    )
    result = run_check("check-unsafe", "--root", str(root))
    assert result.returncode == 1
    assert "crates/rastai-formats/src/safetensors/reader.rs:2: `unsafe` outside the ADR exception list" in result.stdout
    assert "crates/rastai-core/src/ids.rs:1:" in result.stdout
    assert "crates/rastai-diag/src/lib.rs:1:" in result.stdout


def test_a_second_unsafe_in_the_allowed_file_fails(write_tree, run_check):
    root = write_tree({ALLOWED_FILE: ONE_STATVFS_CALL + "fn more() { unsafe { other() } }\n"})
    result = run_check("check-unsafe", "--root", str(root))
    assert result.returncode == 1
    assert f"{ALLOWED_FILE}: 2 `unsafe` uses, but its ADR allows 1" in result.stdout


def test_the_real_repository_passes(run_check):
    result = run_check("check-unsafe")
    assert result.returncode == 0, result.stdout
