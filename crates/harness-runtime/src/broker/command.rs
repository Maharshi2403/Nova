//! Policy-controlled command execution (`cmd.run`).
//!
//! Commands are argv vectors that never touch a shell. A command runs only when its argv
//! starts with an allowlisted prefix, or when that exact argv has been explicitly approved.
//! Anything else is denied with `approval_required` and recorded as pending.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use harness_core::messages::{ToolRequest, ToolResponse};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use super::paths::{PathError, PathPolicy};
use super::write::RepoWriteBroker;
use crate::supervisor::ToolBroker;

pub const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
pub const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(900);
const MAX_ARGV: usize = 64;
const MAX_ARG_BYTES: usize = 8 * 1024;

const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "Path",
    "SystemRoot",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// Not allowlisted and not approved.
    ApprovalRequired,
}

#[derive(Debug, Clone, Default)]
pub struct CommandPolicy {
    allowed_prefixes: Vec<Vec<String>>,
    approved: Vec<Vec<String>>,
}

impl CommandPolicy {
    pub fn new(allowed_prefixes: Vec<Vec<String>>) -> Self {
        Self {
            allowed_prefixes,
            approved: Vec::new(),
        }
    }

    pub fn approve(&mut self, argv: Vec<String>) {
        self.approved.push(argv);
    }

    pub fn decide(&self, argv: &[String]) -> Decision {
        let allowlisted = self
            .allowed_prefixes
            .iter()
            .any(|p| !p.is_empty() && argv.starts_with(p));
        if allowlisted || self.approved.iter().any(|a| a == argv) {
            Decision::Allowed
        } else {
            Decision::ApprovalRequired
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration: Duration,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

async fn capture<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    // Keep draining past the cap so a chatty child never blocks on a full pipe.
    while let Ok(n) = reader.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(kept.len());
        if n > room {
            truncated = true;
        }
        kept.extend_from_slice(&chunk[..n.min(room)]);
    }
    (kept, truncated)
}

/// Terminate the child and everything it spawned.
pub(crate) async fn kill_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
}

/// Run `argv` in `cwd` without a shell, a scrubbed environment, a deadline and capped output.
pub async fn run_command(
    cwd: &Path,
    argv: &[String],
    timeout: Duration,
    max_output: usize,
) -> std::io::Result<CommandOutput> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty argv"))?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    for name in ENV_ALLOWLIST {
        if let Some(v) = std::env::var_os(name) {
            cmd.env(name, v);
        }
    }
    cmd.env("PYTHONUTF8", "1");
    let started = Instant::now();
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_task = tokio::spawn(capture(stdout, max_output));
    let err_task = tokio::spawn(capture(stderr, max_output));

    let (exit_code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => (status?.code(), false),
        Err(_) => {
            kill_tree(&mut child).await;
            (None, true)
        }
    };
    let (out, out_trunc) = out_task.await.unwrap_or_default();
    let (err, err_trunc) = err_task.await.unwrap_or_default();
    Ok(CommandOutput {
        exit_code,
        timed_out,
        duration: started.elapsed(),
        stdout: harness_core::redaction::redact(&String::from_utf8_lossy(&out)),
        stderr: harness_core::redaction::redact(&String::from_utf8_lossy(&err)),
        truncated: out_trunc || err_trunc,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CmdArgs {
    argv: Vec<String>,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
}

fn fail(code: &str, detail: impl std::fmt::Display) -> ToolResponse {
    ToolResponse {
        ok: false,
        result: None,
        error: Some(format!("{code}: {detail}")),
    }
}

/// File tools plus `cmd.run`.
pub struct RepoCommandBroker {
    inner: RepoWriteBroker,
    policy: PathPolicy,
    commands: CommandPolicy,
    allow_cmd: bool,
    pending: Vec<Vec<String>>,
}

impl RepoCommandBroker {
    pub fn new(
        inner: RepoWriteBroker,
        worktree: &Path,
        allowed_tools: &[String],
        commands: CommandPolicy,
    ) -> Result<Self, PathError> {
        Ok(Self {
            inner,
            policy: PathPolicy::new(worktree)?,
            commands,
            allow_cmd: allowed_tools.iter().any(|t| t == "cmd.run"),
            pending: Vec::new(),
        })
    }

    /// Commands that were denied for lack of approval, for the approval workflow.
    pub fn pending_approvals(&self) -> &[Vec<String>] {
        &self.pending
    }

    pub fn approve(&mut self, argv: Vec<String>) {
        self.pending.retain(|p| p != &argv);
        self.commands.approve(argv);
    }

    async fn run(&mut self, request: &ToolRequest) -> ToolResponse {
        let args: CmdArgs = match serde_json::from_value(Value::Object(request.arguments.clone())) {
            Ok(a) => a,
            Err(e) => return fail("bad_arguments", e),
        };
        if args.argv.is_empty()
            || args.argv.len() > MAX_ARGV
            || args
                .argv
                .iter()
                .any(|a| a.len() > MAX_ARG_BYTES || a.contains('\0'))
            || args.argv[0].is_empty()
        {
            return fail(
                "bad_arguments",
                "argv must be 1..=64 non-empty-program strings",
            );
        }
        let cwd = match self
            .policy
            .resolve_existing(args.cwd.as_deref().unwrap_or("."))
        {
            Ok(p) if p.is_dir() => p,
            Ok(_) => return fail("bad_arguments", "cwd is not a directory"),
            Err(e) => return fail("error", e),
        };
        if self.commands.decide(&args.argv) != Decision::Allowed {
            if !self.pending.contains(&args.argv) {
                self.pending.push(args.argv.clone());
            }
            return fail(
                "approval_required",
                "command is not allowlisted or approved",
            );
        }
        let timeout = args
            .timeout_seconds
            .map_or(DEFAULT_COMMAND_TIMEOUT, Duration::from_secs)
            .clamp(Duration::from_secs(1), MAX_COMMAND_TIMEOUT);
        match run_command(&cwd, &args.argv, timeout, MAX_COMMAND_OUTPUT_BYTES).await {
            Ok(o) => ToolResponse {
                ok: true,
                result: Some(json!({
                    "exit_code": o.exit_code,
                    "timed_out": o.timed_out,
                    "duration_ms": u64::try_from(o.duration.as_millis()).unwrap_or(u64::MAX),
                    "stdout": o.stdout,
                    "stderr": o.stderr,
                    "truncated": o.truncated,
                })),
                error: None,
            },
            Err(e) => fail("spawn_failed", e),
        }
    }
}

impl ToolBroker for RepoCommandBroker {
    async fn handle(&mut self, request: &ToolRequest) -> ToolResponse {
        if request.tool == "cmd.run" {
            if !self.allow_cmd {
                return fail("tool_not_allowed", "cmd.run");
            }
            return self.run(request).await;
        }
        self.inner.handle(request).await
    }
}
