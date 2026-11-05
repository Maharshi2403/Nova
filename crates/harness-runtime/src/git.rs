//! Trusted repository registration and a minimal, shell-free Git runner.
//!
//! Registration is read-only: it never stages, resets, cleans or otherwise touches the
//! user's working tree.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_DIRTY_ENTRIES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepoError {
    #[error("repository path must not be empty or contain NUL")]
    InvalidPath,
    #[error("repository path must be absolute: {0}")]
    NotAbsolute(String),
    #[error("repository path does not exist: {0}")]
    NotFound(String),
    #[error("repository path is not a directory: {0}")]
    NotDirectory(String),
    #[error("not a Git repository: {0}")]
    NotGit(String),
    #[error("path is inside a repository but is not its root (root is {root})")]
    NotRepoRoot { root: String },
    #[error("repository has no commits; a base commit is required")]
    NoCommits,
    #[error("working tree has uncommitted changes (first entries: {entries:?})")]
    Dirty { entries: Vec<String> },
    #[error("git could not be run: {0}")]
    GitUnavailable(String),
    #[error("git {args} failed: {stderr}")]
    GitFailed { args: String, stderr: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRepo {
    /// Canonical absolute path of the repository root.
    pub root: PathBuf,
    pub head_sha: String,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
}

/// Canonical form without the Windows `\\?\` verbatim prefix, which many tools reject.
pub fn simplify(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Validate and normalise a user-supplied repository path. Only syntactic and filesystem
/// checks happen here; Git is not consulted.
pub fn validate_repo_path(input: &str) -> Result<PathBuf, RepoError> {
    if input.trim().is_empty() || input.contains('\0') {
        return Err(RepoError::InvalidPath);
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err(RepoError::NotAbsolute(input.to_owned()));
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|_| RepoError::NotFound(input.to_owned()))?;
    if !canonical.is_dir() {
        return Err(RepoError::NotDirectory(input.to_owned()));
    }
    Ok(simplify(canonical))
}

/// Run `git <args>` in `cwd` without a shell, with a scrubbed environment, and return stdout.
pub async fn git(cwd: &Path, args: &[&str]) -> Result<String, RepoError> {
    let mut cmd = Command::new("git");
    // `core.fsmonitor=false` stops a repository's config from launching helper programs
    // during read-only commands.
    cmd.args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=off"])
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for name in [
        "PATH",
        "Path",
        "SystemRoot",
        "SYSTEMROOT",
        "HOME",
        "USERPROFILE",
        "TEMP",
        "TMP",
    ] {
        if let Some(v) = std::env::var_os(name) {
            cmd.env(name, v);
        }
    }
    let output = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| RepoError::GitFailed {
            args: args.join(" "),
            stderr: "timed out".into(),
        })?
        .map_err(|e| RepoError::GitUnavailable(e.to_string()))?;
    if !output.status.success() {
        return Err(RepoError::GitFailed {
            args: args.join(" "),
            stderr: harness_core::redaction::redact(String::from_utf8_lossy(&output.stderr).trim()),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Register `input` as a trusted repository: it must be an existing Git repository root
/// with at least one commit and a clean working tree (untracked files count as dirty).
pub async fn register_repo(input: &str) -> Result<TrustedRepo, RepoError> {
    let root = validate_repo_path(input)?;
    let top = match git(&root, &["rev-parse", "--show-toplevel"]).await {
        Ok(t) => t,
        Err(RepoError::GitFailed { .. }) => return Err(RepoError::NotGit(input.to_owned())),
        Err(e) => return Err(e),
    };
    let top = std::fs::canonicalize(top.trim())
        .map(simplify)
        .map_err(|_| RepoError::NotGit(input.to_owned()))?;
    if top != root {
        return Err(RepoError::NotRepoRoot {
            root: top.display().to_string(),
        });
    }
    let head_sha = match git(&root, &["rev-parse", "--verify", "HEAD^{commit}"]).await {
        Ok(s) => s.trim().to_owned(),
        Err(RepoError::GitFailed { .. }) => return Err(RepoError::NoCommits),
        Err(e) => return Err(e),
    };
    let status = git(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )
    .await?;
    let entries: Vec<String> = status
        .lines()
        .take(MAX_DIRTY_ENTRIES)
        .map(str::to_owned)
        .collect();
    if !entries.is_empty() {
        return Err(RepoError::Dirty { entries });
    }
    let branch = git(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await
        .ok()
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty());
    Ok(TrustedRepo {
        root,
        head_sha,
        branch,
    })
}
