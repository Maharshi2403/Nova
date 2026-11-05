//! Per-attempt Git worktrees on dedicated task branches.
//!
//! Every worktree this manager creates lives under its own `base_dir`, outside the user's
//! repository, and is recorded in a registry file. Cleanup only ever removes registered
//! worktrees whose path is still inside `base_dir`; anything else is refused.

use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::git::{RepoError, TrustedRepo, git, simplify};

const REGISTRY_FILE: &str = "worktrees.json";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorktreeError {
    #[error("invalid {0}: use 1-64 characters of A-Z a-z 0-9 . _ - (not starting with . or -)")]
    BadId(&'static str),
    #[error("base commit must be a full 40-character SHA that exists in the repository")]
    BadBaseSha,
    #[error("worktree base directory must not be inside the repository")]
    BaseInsideRepo,
    #[error("worktree already exists: {0}")]
    AlreadyExists(String),
    #[error("not a harness-owned worktree: {0}")]
    NotOwned(String),
    #[error("registry error: {0}")]
    Registry(String),
    #[error(transparent)]
    Git(#[from] RepoError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub run_id: String,
    pub task_id: String,
    pub attempt: u32,
    pub path: PathBuf,
    pub branch: String,
    pub base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub owned: bool,
}

pub struct WorktreeManager {
    repo_root: PathBuf,
    base_dir: PathBuf,
    registry: Mutex<()>,
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with(['.', '-'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl WorktreeManager {
    /// `base_dir` is created if needed and must lie outside the repository.
    pub fn new(repo: &TrustedRepo, base_dir: &Path) -> Result<Self, WorktreeError> {
        std::fs::create_dir_all(base_dir).map_err(|e| WorktreeError::Registry(e.to_string()))?;
        let base_dir = std::fs::canonicalize(base_dir)
            .map(simplify)
            .map_err(|e| WorktreeError::Registry(e.to_string()))?;
        if base_dir.starts_with(&repo.root) {
            return Err(WorktreeError::BaseInsideRepo);
        }
        Ok(Self {
            repo_root: repo.root.clone(),
            base_dir,
            registry: Mutex::new(()),
        })
    }

    fn registry_path(&self) -> PathBuf {
        self.base_dir.join(REGISTRY_FILE)
    }

    fn load(&self) -> Result<Vec<Worktree>, WorktreeError> {
        match std::fs::read_to_string(self.registry_path()) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|e| WorktreeError::Registry(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(WorktreeError::Registry(e.to_string())),
        }
    }

    fn save(&self, entries: &[Worktree]) -> Result<(), WorktreeError> {
        let text = serde_json::to_string_pretty(entries)
            .map_err(|e| WorktreeError::Registry(e.to_string()))?;
        let tmp = self.base_dir.join(format!("{REGISTRY_FILE}.tmp"));
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, self.registry_path()))
            .map_err(|e| WorktreeError::Registry(e.to_string()))
    }

    /// Create a worktree on a new branch `harness/<run>/<task>-a<attempt>` at `base_sha`.
    pub async fn create(
        &self,
        run_id: &str,
        task_id: &str,
        attempt: u32,
        base_sha: &str,
    ) -> Result<Worktree, WorktreeError> {
        if !valid_id(run_id) {
            return Err(WorktreeError::BadId("run id"));
        }
        if !valid_id(task_id) {
            return Err(WorktreeError::BadId("task id"));
        }
        if base_sha.len() != 40 || !base_sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(WorktreeError::BadBaseSha);
        }
        let spec = format!("{base_sha}^{{commit}}");
        git(&self.repo_root, &["cat-file", "-e", &spec])
            .await
            .map_err(|_| WorktreeError::BadBaseSha)?;

        let branch = format!("harness/{run_id}/{task_id}-a{attempt}");
        let path = self
            .base_dir
            .join(run_id)
            .join(format!("{task_id}-a{attempt}"));
        let _guard = self.registry.lock().await;
        let mut entries = self.load()?;
        if path.exists() || entries.iter().any(|w| w.path == path || w.branch == branch) {
            return Err(WorktreeError::AlreadyExists(path.display().to_string()));
        }
        std::fs::create_dir_all(path.parent().expect("worktree path has a parent"))
            .map_err(|e| WorktreeError::Registry(e.to_string()))?;
        let path_arg = path.to_string_lossy().into_owned();
        git(
            &self.repo_root,
            &[
                "worktree", "add", "--quiet", "-b", &branch, &path_arg, base_sha,
            ],
        )
        .await?;
        let wt = Worktree {
            run_id: run_id.to_owned(),
            task_id: task_id.to_owned(),
            attempt,
            path,
            branch,
            base_sha: base_sha.to_owned(),
        };
        entries.push(wt.clone());
        self.save(&entries)?;
        Ok(wt)
    }

    /// Every worktree Git knows about, flagged by whether this manager owns it.
    pub async fn list(&self) -> Result<Vec<ListedWorktree>, WorktreeError> {
        let out = git(&self.repo_root, &["worktree", "list", "--porcelain"]).await?;
        let owned = self.load()?;
        let mut listed = Vec::new();
        for block in out.split("\n\n").filter(|b| !b.trim().is_empty()) {
            let mut path = None;
            let mut branch = None;
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    path = Some(simplify(PathBuf::from(p)));
                } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                    branch = Some(b.to_owned());
                }
            }
            if let Some(path) = path {
                let is_owned = owned
                    .iter()
                    .any(|w| same_path(&w.path, &path) && path.starts_with(&self.base_dir));
                listed.push(ListedWorktree {
                    path,
                    branch,
                    owned: is_owned,
                });
            }
        }
        Ok(listed)
    }

    /// Remove an owned worktree. The branch is kept unless `delete_branch` is set, so
    /// committed work is never lost by accident.
    pub async fn cleanup(&self, path: &Path, delete_branch: bool) -> Result<(), WorktreeError> {
        let _guard = self.registry.lock().await;
        let mut entries = self.load()?;
        let shown = path.display().to_string();
        let Some(index) = entries.iter().position(|w| same_path(&w.path, path)) else {
            return Err(WorktreeError::NotOwned(shown));
        };
        let wt = entries[index].clone();
        // The registry file lives on disk; never trust it to point outside our directory.
        let under_base = wt
            .path
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .map(simplify)
            .is_some_and(|p| p.starts_with(&self.base_dir));
        if !under_base || wt.path == self.base_dir || !wt.path.starts_with(&self.base_dir) {
            return Err(WorktreeError::NotOwned(shown));
        }
        let path_arg = wt.path.to_string_lossy().into_owned();
        git(
            &self.repo_root,
            &["worktree", "remove", "--force", &path_arg],
        )
        .await?;
        if delete_branch && wt.branch.starts_with("harness/") {
            git(&self.repo_root, &["branch", "-D", &wt.branch]).await?;
        }
        entries.remove(index);
        self.save(&entries)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
