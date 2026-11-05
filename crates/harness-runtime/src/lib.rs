//! Scheduler runtime. Pure decision logic lives here; I/O is added in later tickets.

pub mod broker;
pub mod committer;
pub mod diff_gate;
pub mod git;
pub mod integration;
pub mod priority;
pub mod readiness;
pub mod scheduler;
pub mod supervisor;
pub mod verifier;
pub mod worker;
pub mod worktree;
