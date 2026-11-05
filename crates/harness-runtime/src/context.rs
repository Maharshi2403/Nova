//! Bounded, task-specific retrieval. Ripgrep finds symbol lines; broker policy controls
//! every file read. No repository file is interpreted as an instruction or executable.
use crate::broker::command::run_command;
use crate::broker::paths::{PathError, PathPolicy, is_denied_relative};
use crate::broker::read::RepoReadBroker;
use crate::supervisor::ToolBroker;
use harness_core::messages::ToolRequest;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_INDEX_FILES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    pub path: String,
    pub start_line: u32,
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Context {
    pub snippets: Vec<Snippet>,
    pub truncated: bool,
}

/// Prefer explicit candidate paths. Without them, inspect at most 128 ignored-aware
/// file names and use ripgrep to locate requested symbols, never load the entire repo.
pub async fn retrieve(
    root: &Path,
    candidates: &[String],
    symbols: &[String],
    byte_cap: usize,
) -> Result<Context, PathError> {
    let policy = PathPolicy::new(root)?;
    let mut paths = candidates
        .iter()
        .take(MAX_INDEX_FILES)
        .cloned()
        .collect::<Vec<_>>();
    if paths.is_empty() {
        let base = policy.root().to_path_buf();
        let mut walk = ignore::WalkBuilder::new(&base);
        walk.hidden(false)
            .require_git(false)
            .parents(false)
            .follow_links(false)
            .max_depth(Some(16));
        let filter_base = base.clone();
        walk.filter_entry(move |entry| {
            entry
                .path()
                .strip_prefix(&filter_base)
                .is_ok_and(|p| !is_denied_relative(p))
        });
        for entry in walk.build().flatten() {
            if entry.file_type().is_some_and(|t| t.is_file()) {
                paths.push(
                    entry
                        .path()
                        .strip_prefix(&base)
                        .expect("walk root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                if paths.len() >= MAX_INDEX_FILES {
                    break;
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    let tools = vec!["repo.read".into()];
    let mut broker = RepoReadBroker::new(root, &tools)?;
    let mut result = Context {
        snippets: Vec::new(),
        truncated: candidates.len() > MAX_INDEX_FILES,
    };
    let mut remaining = byte_cap.min(MAX_CONTEXT_BYTES);
    for path in paths {
        if result.snippets.len() >= 8 || remaining == 0 {
            result.truncated = true;
            break;
        }
        let full = match policy.resolve_existing(&path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !full.is_file() || std::fs::metadata(&full).map_or(true, |m| m.len() > 256 * 1024) {
            continue;
        }
        let mut line = 1;
        if !symbols.is_empty() {
            let mut argv = vec![
                "rg".into(),
                "--no-config".into(),
                "--no-heading".into(),
                "--line-number".into(),
                "--max-count".into(),
                "1".into(),
                "--fixed-strings".into(),
            ];
            for symbol in symbols
                .iter()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .take(8)
            {
                argv.extend(["-e".into(), symbol.clone()]);
            }
            // Never allow the absence of patterns to turn a path into the search pattern.
            if !argv.iter().any(|a| a == "-e") {
                continue;
            }
            argv.extend(["--".into(), full.to_string_lossy().into_owned()]);
            match run_command(policy.root(), &argv, Duration::from_secs(2), 4096).await {
                Ok(output) if output.exit_code == Some(0) => {
                    line = output
                        .stdout
                        .split(':')
                        .next()
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(1)
                        .saturating_sub(5)
                        .max(1);
                }
                // Safe fallback when ripgrep is not installed: bounded broker read below.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if candidates.is_empty() {
                        continue;
                    }
                }
                _ => continue,
            }
        }
        let response = broker
            .handle(&ToolRequest {
                tool: "repo.read".into(),
                arguments: json!({"path":path,"start_line":line,"max_lines":40})
                    .as_object()
                    .unwrap()
                    .clone(),
            })
            .await;
        if let Some(value) = response.result {
            let mut content = value["content"].as_str().unwrap_or("").to_string();
            if content.len() > remaining {
                let mut end = remaining;
                while !content.is_char_boundary(end) {
                    end -= 1;
                }
                content.truncate(end);
                result.truncated = true;
            }
            remaining -= content.len();
            result.snippets.push(Snippet {
                path,
                start_line: line,
                content,
            });
        }
    }
    Ok(result)
}
