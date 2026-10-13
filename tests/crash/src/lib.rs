//! rastai-crash-tests: the kill -9 matrix of flows/platform.md 6.5 (contracts 2.1; layout rust-node-gateway-proto.md
//! 7.12). Each row starts a real daemon process with its embedded node, trains TextCNN on the CPU through
//! `train.start`, lets the daemon, the node or the worker stop itself at a named point (`RASTAI_TEST_PAUSE_AT`,
//! rastai-node's pause_points.rs and rastai_worker's pause_points.py), sends SIGKILL, starts the daemon again and
//! checks what the row expects. After every row: every durable record is indexed exactly once, nothing half-written
//! is visible, no worker's process group is left, and every failure carries a catalog code.
//!
//! This library holds nothing: the rows are the integration test `tests/kill_matrix.rs`, one module per row, and
//! `just test-crash` runs them one at a time. The worker's own kill -9 tests are `tests/crash/test_*.py`.
//!
//! M1 rows: 1 (the §32 exit), 2, 3, 9, 11 and 13. The others join with the epics that build what they kill.
