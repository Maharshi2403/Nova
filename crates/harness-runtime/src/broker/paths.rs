//! Path policy for brokered tools: everything a worker names must resolve, after symlink
//! resolution, to a location inside its worktree and must not be Git internals or a
//! well-known secret file.

use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

use crate::git::simplify;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path_denied: {0}")]
    Denied(String),
    #[error("not_found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct PathPolicy {
    root: PathBuf,
}

/// File names that commonly hold credentials. Matching is on the final component.
fn is_sensitive_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let env_file =
        n == ".env" || (n.starts_with(".env.") && n != ".env.example" && n != ".env.sample");
    env_file
        || [".pem", ".key", ".pfx", ".p12"]
            .iter()
            .any(|s| n.ends_with(s))
        || ["id_rsa", "id_ed25519", "id_ecdsa"]
            .iter()
            .any(|s| n.starts_with(s))
        || matches!(
            n.as_str(),
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | ".git-credentials"
                | "credentials"
                | "credentials.json"
        )
}

/// True when any component of a worktree-relative path is off limits.
pub fn is_denied_relative(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            let portable = name.trim_end_matches(['.', ' ']).to_ascii_lowercase();
            matches!(
                portable.as_str(),
                ".git" | ".ssh" | ".aws" | ".azure" | ".config" | ".codex" | ".agents"
            ) || is_sensitive_name(&portable)
        }
        Component::CurDir => false,
        _ => true,
    })
}

impl PathPolicy {
    pub fn new(worktree: &Path) -> Result<Self, PathError> {
        let root = std::fs::canonicalize(worktree)
            .map(simplify)
            .map_err(|_| PathError::NotFound(worktree.display().to_string()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validate a worker-supplied relative path syntactically.
    pub fn check_relative(&self, input: &str) -> Result<PathBuf, PathError> {
        if input.is_empty() || input.chars().any(char::is_control) || input.contains([':', '\\']) {
            return Err(PathError::Denied("empty or invalid path".into()));
        }
        for part in input.split('/') {
            if part == "." {
                continue;
            }
            let name = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            if part.ends_with(['.', ' '])
                || matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (name.len() == 4
                    && (name.starts_with("COM") || name.starts_with("LPT"))
                    && name.as_bytes()[3].is_ascii_digit())
            {
                return Err(PathError::Denied("nonportable or reserved path".into()));
            }
        }
        let normalized: String = input.nfc().collect();
        let rel = Path::new(&normalized);
        if rel.is_absolute() || rel.has_root() || input.starts_with(['/', '\\']) {
            return Err(PathError::Denied("absolute paths are not allowed".into()));
        }
        // Parent, prefix (`C:foo`) and root components all escape the worktree.
        for c in rel.components() {
            match c {
                Component::Normal(_) | Component::CurDir => {}
                _ => {
                    return Err(PathError::Denied(
                        "path must stay inside the worktree".into(),
                    ));
                }
            }
        }
        if is_denied_relative(rel) {
            return Err(PathError::Denied("path is protected".into()));
        }
        Ok(rel.to_path_buf())
    }

    /// Resolve an existing file or directory, following symlinks, and confirm it is still
    /// inside the worktree and not protected.
    pub fn resolve_existing(&self, input: &str) -> Result<PathBuf, PathError> {
        let rel = self.check_relative(input)?;
        let full = std::fs::canonicalize(self.root.join(&rel))
            .map(simplify)
            .map_err(|_| PathError::NotFound(input.to_owned()))?;
        let inside = full
            .strip_prefix(&self.root)
            .map_err(|_| PathError::Denied("path resolves outside the worktree".into()))?;
        if is_denied_relative(inside) {
            return Err(PathError::Denied("path is protected".into()));
        }
        Ok(full)
    }

    /// Resolve a path that may not exist yet for writing. The deepest existing ancestor
    /// and any existing target are canonicalised (so symlinks are followed) and must stay
    /// inside the worktree; an existing symlink target is refused outright.
    pub fn resolve_for_write(&self, input: &str) -> Result<PathBuf, PathError> {
        let rel = self.check_relative(input)?;
        let full = self.root.join(&rel);
        if let Ok(meta) = std::fs::symlink_metadata(&full) {
            if meta.file_type().is_symlink() {
                return Err(PathError::Denied(
                    "refusing to write through a symlink".into(),
                ));
            }
            if !meta.is_file() {
                return Err(PathError::Denied("target is not a regular file".into()));
            }
        }
        let mut ancestor = full.parent();
        while let Some(a) = ancestor {
            if a.exists() {
                let real = std::fs::canonicalize(a)
                    .map(simplify)
                    .map_err(|_| PathError::Denied("cannot resolve parent directory".into()))?;
                let inside = real
                    .strip_prefix(&self.root)
                    .map_err(|_| PathError::Denied("path resolves outside the worktree".into()))?;
                if is_denied_relative(inside) {
                    return Err(PathError::Denied("path is protected".into()));
                }
                return Ok(full);
            }
            ancestor = a.parent();
        }
        Err(PathError::Denied(
            "no existing parent inside the worktree".into(),
        ))
    }
}
