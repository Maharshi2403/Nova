//! Read-only repository tools exposed to workers: `repo.read`, `repo.glob`, `repo.search`.
//!
//! Every result is bounded. Paths are resolved by [`PathPolicy`], and protected files
//! (Git internals, key material, dotenv files) never appear in results.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use globset::GlobBuilder;
use harness_core::messages::{ToolRequest, ToolResponse};
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};

use super::paths::{PathPolicy, is_denied_relative};
use crate::supervisor::ToolBroker;

pub const MAX_SCAN_BYTES: u64 = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_LINES: u32 = 200;
pub const MAX_LINES: u32 = 2000;
pub const MAX_GLOB_RESULTS: usize = 500;
pub const MAX_SEARCH_RESULTS: u32 = 200;
const MAX_MATCH_TEXT: usize = 300;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    start_line: Option<u32>,
    max_lines: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobArgs {
    pattern: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    pattern: String,
    glob: Option<String>,
    max_results: Option<u32>,
    case_insensitive: Option<bool>,
}

pub struct RepoReadBroker {
    policy: PathPolicy,
    allowed: BTreeSet<String>,
}

fn fail(code: &str, detail: impl std::fmt::Display) -> ToolResponse {
    ToolResponse {
        ok: false,
        result: None,
        error: Some(format!("{code}: {detail}")),
    }
}

fn success(value: Value) -> ToolResponse {
    ToolResponse {
        ok: true,
        result: Some(value),
        error: None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(request: &ToolRequest) -> Result<T, ToolResponse> {
    serde_json::from_value(Value::Object(request.arguments.clone()))
        .map_err(|e| fail("bad_arguments", e))
}

/// Cut `text` to at most `max` bytes on a character boundary.
fn clip(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

fn read_bounded(path: &Path) -> Result<(Vec<u8>, bool), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("io_error: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("io_error: {e}"))?;
    if !meta.is_file() {
        return Err("not_a_file: path is not a regular file".into());
    }
    let mut buf = Vec::new();
    file.take(MAX_SCAN_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| format!("io_error: {e}"))?;
    Ok((buf, meta.len() > MAX_SCAN_BYTES))
}

impl RepoReadBroker {
    pub fn new(worktree: &Path, allowed_tools: &[String]) -> Result<Self, super::paths::PathError> {
        Ok(Self {
            policy: PathPolicy::new(worktree)?,
            allowed: allowed_tools.iter().cloned().collect(),
        })
    }

    fn read(&self, request: &ToolRequest) -> ToolResponse {
        let args: ReadArgs = match parse(request) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let full = match self.policy.resolve_existing(&args.path) {
            Ok(p) => p,
            Err(e) => return fail("error", e),
        };
        let (bytes, file_truncated) = match read_bounded(&full) {
            Ok(v) => v,
            Err(e) => return fail("error", e),
        };
        if bytes.contains(&0) {
            return fail("binary_file", "binary files cannot be read as text");
        }
        let start = args.start_line.unwrap_or(1).max(1);
        let max_lines = args
            .max_lines
            .unwrap_or(DEFAULT_MAX_LINES)
            .clamp(1, MAX_LINES);
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let from = (start as usize - 1).min(lines.len());
        let to = (from + max_lines as usize).min(lines.len());
        let mut content = lines[from..to].join("\n");
        let byte_capped = clip(&mut content, MAX_OUTPUT_BYTES);
        let truncated = file_truncated || to < lines.len() || byte_capped;
        success(json!({
            "content": content,
            "start_line": start,
            "end_line": if to > from { to } else { from },
            "truncated": truncated,
        }))
    }

    fn walk(&self) -> WalkBuilder {
        let mut b = WalkBuilder::new(self.policy.root());
        b.hidden(false)
            .require_git(false)
            .parents(false)
            .follow_links(false)
            .max_depth(Some(32));
        let root = self.policy.root().to_path_buf();
        b.filter_entry(move |e| {
            e.path()
                .strip_prefix(&root)
                .map(|rel| !is_denied_relative(rel))
                .unwrap_or(false)
        });
        b
    }

    fn glob(&self, request: &ToolRequest) -> ToolResponse {
        let args: GlobArgs = match parse(request) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let matcher = match GlobBuilder::new(&args.pattern)
            .literal_separator(true)
            .build()
        {
            Ok(g) => g.compile_matcher(),
            Err(e) => return fail("bad_arguments", e),
        };
        let mut paths = BTreeSet::new();
        let mut truncated = false;
        for entry in self.walk().build().flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(self.policy.root()) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if matcher.is_match(&rel) {
                if paths.len() >= MAX_GLOB_RESULTS {
                    truncated = true;
                    break;
                }
                paths.insert(rel);
            }
        }
        success(json!({"paths": paths, "truncated": truncated}))
    }

    fn search(&self, request: &ToolRequest) -> ToolResponse {
        let args: SearchArgs = match parse(request) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let regex = match RegexBuilder::new(&args.pattern)
            .case_insensitive(args.case_insensitive.unwrap_or(false))
            .size_limit(1 << 20)
            .build()
        {
            Ok(r) => r,
            Err(e) => return fail("bad_arguments", e),
        };
        let filter = match args
            .glob
            .as_deref()
            .map(|g| GlobBuilder::new(g).literal_separator(true).build())
        {
            None => None,
            Some(Ok(g)) => Some(g.compile_matcher()),
            Some(Err(e)) => return fail("bad_arguments", e),
        };
        let limit = args.max_results.unwrap_or(100).clamp(1, MAX_SEARCH_RESULTS) as usize;
        let mut matches = Vec::new();
        let mut truncated = false;
        let mut output_bytes = 0usize;
        'files: for entry in self.walk().build().flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(self.policy.root()) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if filter.as_ref().is_some_and(|g| !g.is_match(&rel)) {
                continue;
            }
            let Ok((bytes, _)) = read_bounded(entry.path()) else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            for (i, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if !regex.is_match(line) {
                    continue;
                }
                let mut text = line.to_owned();
                clip(&mut text, MAX_MATCH_TEXT);
                output_bytes += text.len() + rel.len() + 16;
                if matches.len() >= limit || output_bytes > MAX_OUTPUT_BYTES {
                    truncated = true;
                    break 'files;
                }
                matches.push(json!({"path": rel, "line": i + 1, "text": text}));
            }
        }
        success(json!({"matches": matches, "truncated": truncated}))
    }
}

impl ToolBroker for RepoReadBroker {
    async fn handle(&mut self, request: &ToolRequest) -> ToolResponse {
        if !self.allowed.contains(&request.tool) {
            return fail("tool_not_allowed", &request.tool);
        }
        match request.tool.as_str() {
            "repo.read" => self.read(request),
            "repo.glob" => self.glob(request),
            "repo.search" => self.search(request),
            other => fail("unknown_tool", other),
        }
    }
}
