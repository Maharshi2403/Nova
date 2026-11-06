//! Bounded scheduling, supervised workers, tool permissions, verification and Git integration.

pub mod broker;
pub mod budget;
pub mod committer;
pub mod context;
pub mod diff_gate;
pub mod engine;
pub mod git;
pub mod integration;
pub mod live;
pub mod locks;
pub mod orchestrator;
pub mod pool;
pub mod priority;
pub mod provider_config;
pub mod readiness;
pub mod recovery;
pub mod sandbox;
pub mod scheduler;
pub mod supervisor;
pub mod vault;
pub mod verifier;
pub mod worker;
pub mod worktree;

pub mod catalog;
