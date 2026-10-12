//! Python jobs in their own process groups (layout: rust-node-gateway-proto.md section 7.4). Read supervisor.rs
//! first: it owns the job table and the order of a job's life; the other files are its parts.

pub mod exit_cause;
pub mod job_file;
pub mod log_lines;
pub mod reattach;
pub mod signals;
pub mod spawn;
pub mod supervisor;
