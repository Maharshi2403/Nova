//! `repo.patch`: the only way a worker changes files. Writes are limited to the task's
//! assigned paths (glob patterns) inside its worktree; Git internals and secret-looking
//! files are never writable. Read tools are delegated to [`RepoReadBroker`].

use std::io::Write;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use harness_core::messages::{ToolRequest, ToolResponse};
use serde::Deserialize;
use serde_json::{Value, json};

use super::paths::{PathError, PathPolicy};
use super::read::{MAX_SCAN_BYTES, RepoReadBroker};
use crate::supervisor::ToolBroker;

pub const MAX_WRITE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArgs {
    path: String,
    /// Whole-file contents (creates or overwrites).
    content: Option<String>,
    /// Exact-match replacement; `old` must occur exactly once.
    old: Option<String>,
    new: Option<String>,
}

pub struct RepoWriteBroker {
    read: RepoReadBroker,
    policy: PathPolicy,
    allow_patch: bool,
    writable: GlobSet,
}

fn fail(code: &str, detail: impl std::fmt::Display) -> ToolResponse {
    ToolResponse {
        ok: false,
        result: None,
        error: Some(format!("{code}: {detail}")),
    }
}

impl RepoWriteBroker {
    /// `assigned_paths` are the task's candidate paths: exact files, globs, or directories
    /// (a path also matches everything beneath it).
    pub fn new(
        worktree: &Path,
        allowed_tools: &[String],
        assigned_paths: &[String],
    ) -> Result<Self, PathError> {
        let mut builder = GlobSetBuilder::new();
        for p in assigned_paths {
            let normalized: String = p.nfc().collect();
            let p = normalized.trim_end_matches('/');
            for pattern in [p.to_owned(), format!("{p}/**")] {
                let glob = GlobBuilder::new(&pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| PathError::Denied(format!("bad assigned path {p:?}: {e}")))?;
                builder.add(glob);
            }
        }
        Ok(Self {
            read: RepoReadBroker::new(worktree, allowed_tools)?,
            policy: PathPolicy::new(worktree)?,
            allow_patch: allowed_tools.iter().any(|t| t == "repo.patch"),
            writable: builder
                .build()
                .map_err(|e| PathError::Denied(e.to_string()))?,
        })
    }

    fn patch(&self, request: &ToolRequest) -> ToolResponse {
        let args: PatchArgs = match serde_json::from_value(Value::Object(request.arguments.clone()))
        {
            Ok(a) => a,
            Err(e) => return fail("bad_arguments", e),
        };
        let rel = match self.policy.check_relative(&args.path) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(e) => return fail("error", e),
        };
        let rel = rel.trim_start_matches("./").to_owned();
        if !self.writable.is_match(&rel) {
            return fail(
                "error",
                "path_not_assigned: path is outside this task's assigned paths",
            );
        }
        let full = match self.policy.resolve_for_write(&args.path) {
            Ok(p) => p,
            Err(e) => return fail("error", e),
        };

        let new_text = match (&args.content, &args.old, &args.new) {
            (Some(content), None, None) => content.clone(),
            (None, Some(old), Some(new)) => {
                if old.is_empty() {
                    return fail("bad_arguments", "old must not be empty");
                }
                let current = match read_text(&full) {
                    Ok(t) => t,
                    Err(r) => return r,
                };
                match current.matches(old.as_str()).count() {
                    0 => return fail("no_match", "old text not found"),
                    1 => current.replacen(old.as_str(), new, 1),
                    n => return fail("ambiguous_match", format!("old text occurs {n} times")),
                }
            }
            _ => {
                return fail(
                    "bad_arguments",
                    "provide either `content`, or both `old` and `new`",
                );
            }
        };
        if new_text.len() > MAX_WRITE_BYTES {
            return fail(
                "too_large",
                format!("file would exceed {MAX_WRITE_BYTES} bytes"),
            );
        }
        if let Some(parent) = full.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return fail("io_error", e);
        }
        // Recheck after creating directories, then atomically replace the entry.
        // This does not follow a swapped final symlink or overwrite a hardlink's inode.
        // Parent-directory TOCTOU still requires trusted-local or a real OS sandbox.
        if let Err(e) = self.policy.resolve_for_write(&args.path) {
            return fail("error", e);
        }
        if let Err(e) = atomic_write(&full, new_text.as_bytes()) {
            return fail("io_error", e);
        }
        ToolResponse {
            ok: true,
            result: Some(json!({"path": rel, "bytes_written": new_text.len()})),
            error: None,
        }
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_file_name(format!(".harness-write-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_text(path: &Path) -> Result<String, ToolResponse> {
    let meta = std::fs::metadata(path).map_err(|_| fail("not_found", "file does not exist"))?;
    if meta.len() > MAX_SCAN_BYTES {
        return Err(fail("too_large", "file too large for a replacement edit"));
    }
    String::from_utf8(std::fs::read(path).map_err(|e| fail("io_error", e))?)
        .map_err(|_| fail("binary_file", "file is not valid UTF-8"))
}

impl ToolBroker for RepoWriteBroker {
    async fn handle(&mut self, request: &ToolRequest) -> ToolResponse {
        if request.tool == "repo.patch" {
            if !self.allow_patch {
                return fail("tool_not_allowed", "repo.patch");
            }
            return self.patch(request);
        }
        self.read.handle(request).await
    }
}
