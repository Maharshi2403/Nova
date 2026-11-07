//! Serialized, non-destructive integration of verified task commits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use harness_core::domain::{Task, TaskState};
use harness_store::Store;
use harness_store::git_artifacts::GitArtifactRepository;
use harness_store::repository::Repository;
use tokio::sync::Mutex;

use crate::broker::command::{DEFAULT_COMMAND_TIMEOUT, MAX_COMMAND_OUTPUT_BYTES, run_command};
use crate::git::{RepoError, git, simplify};

#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("run was cancelled; integration is not permitted")]
    RunCancelled,
    #[error("task has no verified task commit")]
    MissingTaskCommit,
    #[error("integration merge conflicted; task branch is retained: {0}")]
    Conflict(String),
    #[error("integration check failed; merge is quarantined: {0}")]
    FinalCheckFailed(String),
    #[error("task is not ready to be integrated: {0:?}")]
    InvalidTaskState(TaskState),
    #[error("invalid integration run id")]
    InvalidRunId,
    #[error("integration Git operation failed: {0}")]
    Git(#[from] RepoError),
    #[error("integration command failed to start: {0}")]
    Command(#[from] std::io::Error),
    #[error("integration state persistence failed: {0}")]
    Store(#[from] harness_store::StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationOutcome {
    pub integration_sha: String,
    pub replayed: bool,
}

pub struct Integrator {
    repo_root: PathBuf,
    worktree_path: PathBuf,
    target_ref: String,
    branch: String,
    final_checks: Vec<Vec<String>>,
    lock: Arc<Mutex<()>>,
}

fn project_lock(root: &Path) -> Arc<Mutex<()>> {
    type Locks = std::sync::Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>;
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, v| v.strong_count() > 0);
    if let Some(lock) = locks.get(root).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(root.to_path_buf(), Arc::downgrade(&lock));
    lock
}

impl Integrator {
    pub fn new(
        repo_root: &Path,
        worktree_base: &Path,
        run_id: &str,
        target_ref: &str,
        final_checks: Vec<Vec<String>>,
    ) -> Result<Self, IntegrationError> {
        if run_id.is_empty()
            || run_id.len() > 64
            || run_id.starts_with(['.', '-'])
            || !run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(IntegrationError::InvalidRunId);
        }
        let repo_root = std::fs::canonicalize(repo_root)
            .map(simplify)
            .map_err(|e| RepoError::NotFound(e.to_string()))?;
        std::fs::create_dir_all(worktree_base)
            .map_err(|e| RepoError::GitUnavailable(e.to_string()))?;
        let worktree_base = std::fs::canonicalize(worktree_base)
            .map(simplify)
            .map_err(|e| RepoError::GitUnavailable(e.to_string()))?;
        if worktree_base.starts_with(&repo_root) {
            return Err(IntegrationError::Git(RepoError::NotRepoRoot {
                root: "integration worktree must be outside the repository".into(),
            }));
        }
        let lock = project_lock(&repo_root);
        Ok(Self {
            repo_root,
            worktree_path: worktree_base.join(format!("integration-{run_id}")),
            target_ref: target_ref.to_owned(),
            branch: format!("harness/integration/{run_id}"),
            final_checks,
            lock,
        })
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }
    pub fn with_checks(&self, final_checks: Vec<Vec<String>>) -> Self {
        Self {
            repo_root: self.repo_root.clone(),
            worktree_path: self.worktree_path.clone(),
            target_ref: self.target_ref.clone(),
            branch: self.branch.clone(),
            final_checks,
            lock: self.lock.clone(),
        }
    }

    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    pub async fn integrate(
        &self,
        store: &Store,
        task: &Task,
    ) -> Result<IntegrationOutcome, IntegrationError> {
        let _guard = self.lock.lock().await;
        if store.get_run(&task.run_id).await?.state == harness_core::domain::RunState::Cancelled {
            return Err(IntegrationError::RunCancelled);
        }
        let current = store.get_task(&task.run_id, task.id()).await?;
        if !matches!(
            current.state,
            TaskState::Reviewing | TaskState::Integrating | TaskState::Completed
        ) {
            return Err(IntegrationError::InvalidTaskState(current.state));
        }
        let record = store
            .get_task_commit(&task.run_id, task.id())
            .await?
            .ok_or(IntegrationError::MissingTaskCommit)?;
        if let Some(sha) = record.integration_commit_sha {
            return self.complete_task(store, task, sha, true).await;
        }
        self.ensure_worktree().await?;
        let pre_merge_sha = git(
            &self.worktree_path,
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )
        .await?
        .trim()
        .to_owned();
        if !self.contains_commit(&record.task_commit_sha).await? {
            let output = run_command(
                &self.worktree_path,
                &[
                    "git".into(),
                    "-c".into(),
                    "core.fsmonitor=false".into(),
                    "-c".into(),
                    "core.hooksPath=".into(),
                    "-c".into(),
                    "commit.gpgsign=false".into(),
                    "merge".into(),
                    "--no-ff".into(),
                    "--no-edit".into(),
                    record.task_commit_sha.clone(),
                ],
                DEFAULT_COMMAND_TIMEOUT,
                MAX_COMMAND_OUTPUT_BYTES,
            )
            .await?;
            if output.timed_out || output.exit_code != Some(0) {
                let detail = format!("{}{}", output.stdout, output.stderr)
                    .trim()
                    .to_owned();
                let _ = git(&self.worktree_path, &["merge", "--abort"]).await;
                return Err(IntegrationError::Conflict(detail));
            }
        }
        for check in &self.final_checks {
            let output = run_command(
                &self.worktree_path,
                check,
                DEFAULT_COMMAND_TIMEOUT,
                MAX_COMMAND_OUTPUT_BYTES,
            )
            .await;
            // The worktree is clean after the merge, so anything untracked came from checks.
            let cleanup = crate::diff_gate::remove_check_artifacts(&self.worktree_path, &[]).await;
            let detail = match output {
                Ok(output) if output.timed_out => Some("timed out".to_owned()),
                Ok(output) if output.exit_code != Some(0) => Some(
                    format!("{}{}", output.stdout, output.stderr)
                        .trim()
                        .to_owned(),
                ),
                Err(e) => Some(format!("check failed to start: {e}")),
                Ok(_) => cleanup
                    .as_ref()
                    .err()
                    .map(|e| format!("check cleanup failed: {e}")),
            };
            if let Some(detail) = detail {
                let merge_sha = git(
                    &self.worktree_path,
                    &["rev-parse", "--verify", "HEAD^{commit}"],
                )
                .await?
                .trim()
                .to_owned();
                let quarantine_ref = format!(
                    "refs/harness/quarantine/{}/{}",
                    ref_component(task.run_id.as_str()),
                    ref_component(task.id().as_str())
                );
                git(
                    &self.worktree_path,
                    &["update-ref", &quarantine_ref, &merge_sha],
                )
                .await?;
                let evidence = store
                    .record_task_quarantine(
                        &task.run_id,
                        task.id(),
                        &quarantine_ref,
                        &merge_sha,
                        &detail,
                    )
                    .await;
                git(&self.worktree_path, &["reset", "--hard", &pre_merge_sha]).await?;
                evidence?;
                return Err(IntegrationError::FinalCheckFailed(detail));
            }
        }
        let sha = git(
            &self.worktree_path,
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )
        .await?
        .trim()
        .to_owned();
        store
            .record_integration_commit(&task.run_id, task.id(), &sha)
            .await?;
        self.complete_task(store, task, sha, false).await
    }

    async fn ensure_worktree(&self) -> Result<(), IntegrationError> {
        if self.worktree_path.exists() {
            let branch = git(&self.worktree_path, &["branch", "--show-current"]).await?;
            let top = git(&self.worktree_path, &["rev-parse", "--show-toplevel"]).await?;
            let root = std::fs::canonicalize(top.trim()).map(simplify)?;
            let expected = std::fs::canonicalize(&self.worktree_path).map(simplify)?;
            if branch.trim() != self.branch
                || root != expected
                || !git(&self.worktree_path, &["status", "--porcelain"])
                    .await?
                    .trim()
                    .is_empty()
            {
                return Err(IntegrationError::Conflict(
                    "integration worktree identity changed or has uncommitted changes".into(),
                ));
            }
            return Ok(());
        }
        let path = self.worktree_path.to_string_lossy().into_owned();
        git(
            &self.repo_root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                &self.branch,
                &path,
                &self.target_ref,
            ],
        )
        .await?;
        Ok(())
    }

    async fn contains_commit(&self, commit: &str) -> Result<bool, IntegrationError> {
        let output = run_command(
            &self.worktree_path,
            &[
                "git".into(),
                "-c".into(),
                "core.fsmonitor=false".into(),
                "merge-base".into(),
                "--is-ancestor".into(),
                commit.into(),
                "HEAD".into(),
            ],
            DEFAULT_COMMAND_TIMEOUT,
            MAX_COMMAND_OUTPUT_BYTES,
        )
        .await?;
        if output.timed_out || !matches!(output.exit_code, Some(0 | 1)) {
            return Err(IntegrationError::Git(RepoError::GitFailed {
                args: "merge-base --is-ancestor".into(),
                stderr: format!("{}{}", output.stdout, output.stderr),
            }));
        }
        Ok(output.exit_code == Some(0))
    }

    async fn complete_task(
        &self,
        store: &Store,
        task: &Task,
        sha: String,
        replayed: bool,
    ) -> Result<IntegrationOutcome, IntegrationError> {
        let current = store.get_task(&task.run_id, task.id()).await?;
        if store.get_run(&task.run_id).await?.state == harness_core::domain::RunState::Cancelled {
            return Err(IntegrationError::RunCancelled);
        }
        match current.state {
            TaskState::Reviewing => {
                let integrating = store
                    .transition_task(
                        &task.run_id,
                        task.id(),
                        current.revision,
                        TaskState::Integrating,
                        Some(&format!(
                            "integration:{}:{}/{}",
                            "start",
                            task.run_id,
                            task.id()
                        )),
                    )
                    .await?;
                store
                    .transition_task(
                        &task.run_id,
                        task.id(),
                        integrating.task.revision,
                        TaskState::Completed,
                        Some(&format!(
                            "integration:{}:{}/{}",
                            "complete",
                            task.run_id,
                            task.id()
                        )),
                    )
                    .await?;
            }
            TaskState::Integrating => {
                store
                    .transition_task(
                        &task.run_id,
                        task.id(),
                        current.revision,
                        TaskState::Completed,
                        Some(&format!(
                            "integration:{}:{}/{}",
                            "complete",
                            task.run_id,
                            task.id()
                        )),
                    )
                    .await?;
            }
            TaskState::Completed => {}
            other => return Err(IntegrationError::InvalidTaskState(other)),
        }
        Ok(IntegrationOutcome {
            integration_sha: sha,
            replayed,
        })
    }
}

fn ref_component(value: &str) -> String {
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        value.to_owned()
    } else {
        let mut encoded = String::from("id-");
        for byte in value.bytes() {
            encoded.push_str(&format!("{byte:02x}"));
        }
        encoded
    }
}
