//! Task-local Git commits, made idempotent by durable evidence and a Git trailer.

use std::path::Path;

use harness_core::domain::Task;
use harness_store::Store;
use harness_store::git_artifacts::{GitArtifactRepository, TaskCommitRecord};

use crate::diff_gate::{DiffGate, DiffGateError};
use crate::git::{RepoError, git};
use crate::verifier::{VerificationError, Verifier};

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("verification failed; no commit was created")]
    VerificationFailed,
    #[error("diff requires review before committing")]
    ReviewRequired,
    #[error("diff gate failed: {0}")]
    Diff(#[from] DiffGateError),
    #[error("verification failed to run: {0}")]
    Verify(#[from] VerificationError),
    #[error("git commit operation failed: {0}")]
    Git(#[from] RepoError),
    #[error("could not persist task commit: {0}")]
    Store(#[from] harness_store::StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    pub sha: String,
    pub replayed: bool,
}

pub struct TaskCommitter<'a> {
    verifier: &'a Verifier,
    diff_gate: &'a DiffGate,
}

impl<'a> TaskCommitter<'a> {
    pub fn new(verifier: &'a Verifier, diff_gate: &'a DiffGate) -> Self {
        Self {
            verifier,
            diff_gate,
        }
    }

    pub async fn commit(
        &self,
        store: &Store,
        task: &Task,
        worktree: &Path,
    ) -> Result<CommitOutcome, CommitError> {
        if let Some(record) = store.get_task_commit(&task.run_id, task.id()).await? {
            return Ok(CommitOutcome {
                sha: record.task_commit_sha,
                replayed: true,
            });
        }
        let marker = format!("Harness-Task: {}/{}", task.run_id, task.id());
        let attempt_marker = format!("Harness-Attempt: {}", task.attempt);
        if let Some(sha) = existing_commit(worktree, &marker, &attempt_marker).await? {
            // Crash after Git commit but before SQLite acknowledgement. A trailer is
            // not verification evidence: rerun checks and inspect the committed diff.
            let diff = self
                .diff_gate
                .check_against(task, worktree, "HEAD^")
                .await?;
            if diff.requires_review {
                return Err(CommitError::ReviewRequired);
            }
            if !self.verifier.verify(store, task, worktree).await?.passed {
                return Err(CommitError::VerificationFailed);
            }
            store
                .record_task_commit(&TaskCommitRecord {
                    run_id: task.run_id.clone(),
                    task_id: task.id().clone(),
                    task_commit_sha: sha.clone(),
                    integration_commit_sha: None,
                    diff_paths: diff.changed_paths,
                })
                .await?;
            return Ok(CommitOutcome {
                sha,
                replayed: true,
            });
        }

        let verification = self.verifier.verify(store, task, worktree).await?;
        if !verification.passed {
            return Err(CommitError::VerificationFailed);
        }
        let diff = self.diff_gate.check(task, worktree).await?;
        if diff.requires_review {
            return Err(CommitError::ReviewRequired);
        }
        if diff.changed_paths.is_empty() {
            return Err(CommitError::Git(RepoError::GitFailed {
                args: "commit".into(),
                stderr: "no task changes to commit".into(),
            }));
        }
        let mut add = vec!["add", "--"];
        add.extend(diff.changed_paths.iter().map(String::as_str));
        git(worktree, &add).await?;
        let message = format!("harness({}): {}", task.id(), task.spec.title);
        git(
            worktree,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-verify",
                "-m",
                &message,
                "-m",
                &marker,
                "-m",
                &attempt_marker,
            ],
        )
        .await?;
        let sha = git(worktree, &["rev-parse", "--verify", "HEAD^{commit}"])
            .await?
            .trim()
            .to_owned();
        store
            .record_task_commit(&TaskCommitRecord {
                run_id: task.run_id.clone(),
                task_id: task.id().clone(),
                task_commit_sha: sha.clone(),
                integration_commit_sha: None,
                diff_paths: diff.changed_paths,
            })
            .await?;
        Ok(CommitOutcome {
            sha,
            replayed: false,
        })
    }
}

async fn existing_commit(
    worktree: &Path,
    marker: &str,
    attempt: &str,
) -> Result<Option<String>, RepoError> {
    let body = git(worktree, &["show", "-s", "--format=%B", "HEAD"]).await?;
    if body.lines().any(|line| line == marker) && body.lines().any(|line| line == attempt) {
        return Ok(Some(
            git(worktree, &["rev-parse", "HEAD"])
                .await?
                .trim()
                .to_owned(),
        ));
    }
    Ok(None)
}
