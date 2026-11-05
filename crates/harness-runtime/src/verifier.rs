//! Deterministic execution of task-declared acceptance tests.

use std::path::Path;
use std::time::Duration;

use harness_core::domain::{Acceptance, Task};
use harness_store::verification::{NewTestResult, TestResult, VerificationRepository};
use harness_store::{Store, StoreError};
use serde_json::json;

use crate::broker::command::{DEFAULT_COMMAND_TIMEOUT, MAX_COMMAND_OUTPUT_BYTES, run_command};

#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("declared test command is not allowlisted: {0:?}")]
    CommandNotAllowed(Vec<String>),
    #[error("task declares a non-executable assertion: {0}")]
    UnsupportedAssertion(String),
    #[error("test command could not run: {0}")]
    Command(#[from] std::io::Error),
    #[error("could not persist verification evidence: {0}")]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationOutcome {
    pub passed: bool,
    pub results: Vec<TestResult>,
}

pub struct Verifier {
    allowed_prefixes: Vec<Vec<String>>,
    timeout: Duration,
}

impl Verifier {
    pub fn new(allowed_prefixes: Vec<Vec<String>>) -> Self {
        Self {
            allowed_prefixes,
            timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn with_timeout(allowed_prefixes: Vec<Vec<String>>, timeout: Duration) -> Self {
        Self {
            allowed_prefixes,
            timeout,
        }
    }

    fn allows(&self, command: &[String]) -> bool {
        self.allowed_prefixes
            .iter()
            .any(|prefix| !prefix.is_empty() && command.starts_with(prefix))
    }

    /// Execute each declared test and record its actual result before returning. A failing
    /// command still produces evidence and makes the overall outcome fail.
    pub async fn verify(
        &self,
        store: &Store,
        task: &Task,
        worktree: &Path,
    ) -> Result<VerificationOutcome, VerificationError> {
        let mut results = Vec::new();
        for acceptance in &task.spec.acceptance {
            let Acceptance::Test { command } = acceptance else {
                let Acceptance::Assertion { description } = acceptance else {
                    unreachable!();
                };
                return Err(VerificationError::UnsupportedAssertion(description.clone()));
            };
            if !self.allows(command) {
                return Err(VerificationError::CommandNotAllowed(command.clone()));
            }
            let output =
                run_command(worktree, command, self.timeout, MAX_COMMAND_OUTPUT_BYTES).await?;
            let duration_ms = u64::try_from(output.duration.as_millis()).unwrap_or(u64::MAX);
            let passed = !output.timed_out && output.exit_code == Some(0);
            let result = store
                .record_test_result(NewTestResult {
                    run_id: task.run_id.clone(),
                    task_id: task.id().clone(),
                    command: command.clone(),
                    exit_code: output.exit_code,
                    duration_ms,
                    artifact: json!({
                        "stdout": output.stdout,
                        "stderr": output.stderr,
                        "timed_out": output.timed_out,
                        "truncated": output.truncated,
                    }),
                })
                .await?;
            results.push(result);
            if !passed {
                return Ok(VerificationOutcome {
                    passed: false,
                    results,
                });
            }
        }
        Ok(VerificationOutcome {
            passed: true,
            results,
        })
    }
}
