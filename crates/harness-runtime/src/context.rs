//! Bounded, task-specific retrieval. Ripgrep finds symbol lines; broker policy controls
//! every file read. No repository file is interpreted as an instruction or executable.
use crate::broker::command::run_command;
use crate::broker::paths::{PathError, PathPolicy, is_denied_relative};
use crate::broker::read::RepoReadBroker;
use crate::supervisor::ToolBroker;
use harness_core::messages::ToolRequest;
use serde::Serialize;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_INDEX_FILES: usize = 128;
const MAX_FALLBACK_FILE_BYTES: u64 = 2 * 1024 * 1024;

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
    retrieve_with(root, candidates, symbols, byte_cap, true).await
}

/// `use_ripgrep = false` forces the bounded Rust scanner for deterministic tests.
pub async fn retrieve_with(
    root: &Path,
    candidates: &[String],
    symbols: &[String],
    byte_cap: usize,
    use_ripgrep: bool,
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
        let Ok(metadata) = std::fs::metadata(&full) else {
            continue;
        };
        if !full.is_file() || metadata.len() > MAX_FALLBACK_FILE_BYTES {
            continue;
        }
        let mut line = 1;
        let mut fallback_content = None;
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
            let search = if use_ripgrep && metadata.len() <= 256 * 1024 {
                run_command(policy.root(), &argv, Duration::from_secs(2), 4096).await
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "ripgrep disabled",
                ))
            };
            match search {
                Ok(output) if output.exit_code == Some(0) => {
                    line = output
                        .stdout
                        .split(':')
                        .next()
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(1)
                        .saturating_sub(20)
                        .max(1);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // The canonical path has passed the same deny, secret and symlink
                    // checks as repo.read. Bound the scan even if the file grows after
                    // metadata inspection, then retain only the matching window.
                    let Ok(file) = std::fs::File::open(&full) else {
                        continue;
                    };
                    let mut bytes = Vec::new();
                    if file
                        .take(MAX_FALLBACK_FILE_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .is_err()
                        || bytes.len() as u64 > MAX_FALLBACK_FILE_BYTES
                        || bytes.contains(&0)
                    {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    let lines: Vec<&str> = text.lines().collect();
                    let Some(index) = lines.iter().position(|source_line| {
                        symbols
                            .iter()
                            .filter(|symbol| !symbol.is_empty() && symbol.len() <= 128)
                            .take(8)
                            .any(|symbol| source_line.contains(symbol))
                    }) else {
                        continue;
                    };
                    let from = index.saturating_sub(20);
                    let to = (index + 21).min(lines.len());
                    line = u32::try_from(from + 1).unwrap_or(u32::MAX);
                    fallback_content = Some(lines[from..to].join("\n"));
                }
                _ => continue,
            }
        }
        let content = if let Some(content) = fallback_content {
            Some(content)
        } else {
            broker
                .handle(&ToolRequest {
                    tool: "repo.read".into(),
                    arguments: json!({"path":path,"start_line":line,"max_lines":41})
                        .as_object()
                        .unwrap()
                        .clone(),
                })
                .await
                .result
                .and_then(|value| value["content"].as_str().map(str::to_owned))
        };
        if let Some(mut content) = content {
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
