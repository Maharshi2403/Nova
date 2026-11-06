//! Deterministic pre-commit checks for a task worktree.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use harness_core::domain::{Risk, Task};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::broker::command::{DEFAULT_COMMAND_TIMEOUT, MAX_COMMAND_OUTPUT_BYTES, run_command};
use crate::broker::paths::{PathPolicy, is_denied_relative};
use crate::git::{RepoError, git};

pub const DEFAULT_MAX_DIFF_BYTES: u64 = 1024 * 1024;
const MAX_SECRET_SCAN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DiffGateError {
    #[error("invalid assigned path policy: {0}")]
    InvalidScope(String),
    #[error("changed path is outside the task scope: {0}")]
    UnexpectedPath(String),
    #[error("changed path is protected: {0}")]
    ProtectedPath(String),
    #[error("git diff --check failed: {0}")]
    Whitespace(String),
    #[error("potential secret detected in changed file: {0}")]
    SecretDetected(String),
    #[error("could not read changed file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("could not inspect Git worktree: {0}")]
    Git(#[from] RepoError),
    #[error("could not run git diff --check: {0}")]
    Command(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffGateResult {
    pub changed_paths: Vec<String>,
    pub changed_bytes: u64,
    pub requires_review: bool,
}

pub struct DiffGate {
    scope: GlobSet,
    max_diff_bytes: u64,
    secret_pattern: Regex,
}

impl DiffGate {
    pub fn new(assigned_paths: &[String]) -> Result<Self, DiffGateError> {
        Self::with_max_diff_bytes(assigned_paths, DEFAULT_MAX_DIFF_BYTES)
    }

    pub fn with_max_diff_bytes(
        assigned_paths: &[String],
        max_diff_bytes: u64,
    ) -> Result<Self, DiffGateError> {
        let mut builder = GlobSetBuilder::new();
        for path in assigned_paths {
            let normalized: String = path.nfc().collect();
            let path = normalized.trim_end_matches('/');
            if path.is_empty() {
                return Err(DiffGateError::InvalidScope("empty assigned path".into()));
            }
            for pattern in [path.to_owned(), format!("{path}/**")] {
                builder.add(
                    GlobBuilder::new(&pattern)
                        .literal_separator(true)
                        .build()
                        .map_err(|e| DiffGateError::InvalidScope(e.to_string()))?,
                );
            }
        }
        Ok(Self {
            scope: builder
                .build()
                .map_err(|e| DiffGateError::InvalidScope(e.to_string()))?,
            max_diff_bytes,
            secret_pattern: Regex::new(
                r#"(?i)\b(api[_-]?key|access[_-]?token|secret|password)\b\s*[:=]\s*["']?[^\s"']{8,}"#,
            )
            .expect("static secret pattern is valid"),
        })
    }

    /// Validate the complete current worktree diff against the task's declared scope.
    pub async fn check(
        &self,
        task: &Task,
        worktree: &Path,
    ) -> Result<DiffGateResult, DiffGateError> {
        self.check_against(task, worktree, "HEAD").await
    }

    pub async fn check_against(
        &self,
        task: &Task,
        worktree: &Path,
        base: &str,
    ) -> Result<DiffGateResult, DiffGateError> {
        let policy =
            PathPolicy::new(worktree).map_err(|e| DiffGateError::InvalidScope(e.to_string()))?;
        let mut changed_paths = changed_paths(worktree).await?;
        if base != "HEAD" {
            changed_paths.extend(
                git(
                    worktree,
                    &["diff", "--name-only", "-z", "--no-ext-diff", base, "--"],
                )
                .await?
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
            );
            changed_paths.sort();
            changed_paths.dedup();
        }
        let mut changed_bytes: u64 = 0;
        for path in &changed_paths {
            let relative = policy
                .check_relative(path)
                .map_err(|_| DiffGateError::ProtectedPath(path.clone()))?;
            if is_denied_relative(&relative) {
                return Err(DiffGateError::ProtectedPath(path.clone()));
            }
            let normalized = relative.to_string_lossy().replace('\\', "/");
            if !self.scope.is_match(&normalized) {
                return Err(DiffGateError::UnexpectedPath(normalized));
            }
            let full = worktree.join(&relative);
            if std::fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(DiffGateError::ProtectedPath(normalized));
            }
            if full.exists() {
                policy
                    .resolve_existing(path)
                    .map_err(|_| DiffGateError::ProtectedPath(normalized.clone()))?;
            }
            if let Ok(metadata) = std::fs::metadata(&full) {
                changed_bytes = changed_bytes.saturating_add(metadata.len());
                if metadata.is_file() && metadata.len() <= MAX_SECRET_SCAN_BYTES {
                    let contents = std::fs::read(&full).map_err(|source| DiffGateError::Read {
                        path: normalized.clone(),
                        source,
                    })?;
                    let contents = String::from_utf8_lossy(&contents);
                    if self.secret_pattern.is_match(&contents) {
                        return Err(DiffGateError::SecretDetected(normalized));
                    }
                }
            }
        }

        let command = vec![
            "git".to_owned(),
            "-c".to_owned(),
            "core.fsmonitor=false".to_owned(),
            "diff".to_owned(),
            "--check".to_owned(),
            "--no-ext-diff".to_owned(),
            base.to_owned(),
        ];
        let output = run_command(
            worktree,
            &command,
            DEFAULT_COMMAND_TIMEOUT,
            MAX_COMMAND_OUTPUT_BYTES,
        )
        .await?;
        if output.timed_out || output.exit_code != Some(0) {
            let detail = if output.timed_out {
                "timed out".to_owned()
            } else {
                format!("{}{}", output.stdout, output.stderr)
                    .trim()
                    .to_owned()
            };
            return Err(DiffGateError::Whitespace(detail));
        }
        Ok(DiffGateResult {
            changed_paths,
            changed_bytes,
            requires_review: task.spec.risk == Risk::High || changed_bytes > self.max_diff_bytes,
        })
    }
}

/// Untracked files (not ignored) currently present in `worktree`, sorted.
pub async fn untracked_paths(worktree: &Path) -> Result<Vec<String>, RepoError> {
    let status = git(
        worktree,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )
    .await?;
    let mut paths: Vec<String> = status
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("?? "))
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect();
    paths.sort_unstable();
    Ok(paths)
}

/// Remove untracked files that appeared after `before` was captured. Checks are evidence,
/// not task output: caches such as `__pycache__` must not reach the diff gate or commit.
pub async fn remove_check_artifacts(worktree: &Path, before: &[String]) -> Result<(), RepoError> {
    let created: Vec<String> = untracked_paths(worktree)
        .await?
        .into_iter()
        .filter(|p| before.binary_search(p).is_err())
        .collect();
    for batch in created.chunks(256) {
        let mut args = vec!["clean", "-f", "-q", "--"];
        args.extend(batch.iter().map(String::as_str));
        git(worktree, &args).await?;
    }
    Ok(())
}

async fn changed_paths(worktree: &Path) -> Result<Vec<String>, RepoError> {
    let status = git(
        worktree,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )
    .await?;
    let mut paths = Vec::new();
    for entry in status.split('\0').filter(|entry| !entry.is_empty()) {
        let path = entry
            .get(3..)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| RepoError::GitFailed {
                args: "status --porcelain=v1".into(),
                stderr: format!("malformed status entry {entry:?}"),
            })?;
        paths.push(PathBuf::from(path).to_string_lossy().replace('\\', "/"));
    }
    paths.sort_unstable();
    paths.dedup();
    Ok(paths)
}
