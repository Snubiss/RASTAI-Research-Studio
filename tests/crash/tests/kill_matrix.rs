//! The kill -9 matrix for the skeleton (flows/platform.md 6.5; plan M1-48): rows 1, 2, 3, 9, 11 and 13, one module
//! each, on real processes.
//!
//! - **The daemon is the real `rastai serve --headless`**, a process of its own with `RASTAI_HOME` naming the row's
//!   folder. The rows send it commands over daemon.sock, as the CLI does, and SIGKILL it.
//! - **Every kill lands at a named point.** `RASTAI_TEST_PAUSE_AT` makes the daemon, its embedded node or the worker
//!   stop itself (SIGSTOP) at that point; the row waits for the stop, then kills.
//! - **Every wait is a bounded poll** for a condition, with limits generous enough for a loaded machine; a hang fails
//!   the row with the processes' output.
//! - **Nothing outlives a row.** Dropping a row's daemon kills it, and dropping its scenario kills every worker
//!   process group its runs recorded, on every exit path, a failed assertion included (issue #244).
//!
//! The rows run the real worker in the cpu environment's Python (`just env-cpu`), which a plain `cargo test` on a CI
//! runner doesn't have, and the debug `rastai` binary, which a plain `cargo test` doesn't build, so they are
//! `#[ignore]`d and `just test-crash` builds the binary and runs them, one at a time. Linux only: the rows read /proc
//! and signal with procps' `kill`, as the node does.

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod kill_matrix {
    mod checks;
    mod commands;
    mod daemon_process;
    mod processes;
    mod scenario;

    mod daemon_kill_before_commit;
    mod daemon_kill_embedded_node;
    mod daemon_kill_mid_command;
    mod daemon_kill_worker_finishes;
    mod worker_kill_mid_frame;
    mod worker_kill_normal_run;
}
