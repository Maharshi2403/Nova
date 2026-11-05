//! Supervised Python worker process: spawn without a shell, JSONL over stdio, bounded
//! stderr capture, timeouts and a structured exit report.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harness_core::framing::{FrameError, MAX_FRAME_BYTES, decode_frame, encode_frame};
use harness_core::handshake::{Accepted, HandshakeError, build_hello, verify_ack};
use harness_core::protocol::Envelope;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Newest stderr bytes kept per worker.
pub const STDERR_CAPTURE_BYTES: usize = 64 * 1024;

/// Environment variables forwarded to workers by default. Everything else is dropped;
/// callers add what a worker really needs through [`WorkerSpec::env`].
const DEFAULT_ENV_ALLOWLIST: &[&str] = &["SystemRoot", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"];

#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    /// Extra variables on top of the minimal allowlist.
    pub env: Vec<(String, String)>,
}

impl WorkerSpec {
    /// `<python> -m harness_ai worker`
    pub fn python_worker(python: impl Into<PathBuf>) -> Self {
        Self {
            program: python.into(),
            args: ["-I", "-m", "harness_ai", "worker"]
                .map(OsString::from)
                .to_vec(),
            cwd: None,
            env: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitReport {
    /// `None` when the process was killed by a signal or by us.
    pub code: Option<i32>,
    pub stderr: String,
    pub killed_by_supervisor: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("failed to start worker: {0}")]
    Spawn(std::io::Error),
    #[error("worker exited (code {:?}): {}", .0.code, .0.stderr.trim())]
    Exited(ExitReport),
    #[error("worker did not respond within {0:?}")]
    Timeout(Duration),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
    #[error("io error: {0}")]
    Io(std::io::Error),
}

pub struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_task: Option<JoinHandle<()>>,
    killed: bool,
}

impl Worker {
    pub fn spawn(spec: &WorkerSpec) -> Result<Self, WorkerError> {
        // argv only: no shell ever interprets these strings.
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        for name in DEFAULT_ENV_ALLOWLIST {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        cmd.env("PYTHONUTF8", "1").env("PYTHONUNBUFFERED", "1");
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn().map_err(WorkerError::Spawn)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");

        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&stderr);
        let stderr_task = tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr_pipe.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                let mut buf = sink.lock().expect("stderr buffer poisoned");
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > STDERR_CAPTURE_BYTES {
                    let excess = buf.len() - STDERR_CAPTURE_BYTES;
                    buf.drain(..excess);
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
            stderr_task: Some(stderr_task),
            killed: false,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn send(&mut self, envelope: &Envelope) -> Result<(), WorkerError> {
        let bytes = encode_frame(envelope)?;
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(self.exit_report_error().await);
        };
        let written = async {
            stdin.write_all(&bytes).await?;
            stdin.flush().await
        }
        .await;
        match written {
            Ok(()) => Ok(()),
            // A dead child shows up as a broken pipe; report why it died instead.
            Err(_) => Err(self.exit_report_error().await),
        }
    }

    /// Next frame from the worker; `Ok(None)` means stdout closed.
    pub async fn recv(&mut self, timeout: Duration) -> Result<Option<Envelope>, WorkerError> {
        match tokio::time::timeout(timeout, read_frame_async(&mut self.stdout)).await {
            Ok(result) => Ok(result?),
            Err(_) => Err(WorkerError::Timeout(timeout)),
        }
    }

    /// Run the `hello`/`hello_ack` exchange. On timeout the worker is killed.
    pub async fn handshake(&mut self, timeout: Duration) -> Result<Accepted, WorkerError> {
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("RFC 3339 formatting of the current time cannot fail");
        let hello = build_hello(Uuid::new_v4(), timestamp);
        self.send(&hello).await?;
        match self.recv(timeout).await {
            Ok(Some(ack)) => Ok(verify_ack(&hello, &ack)?),
            Ok(None) => Err(self.exit_report_error().await),
            Err(e @ WorkerError::Timeout(_)) => {
                self.kill().await;
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn kill(&mut self) {
        self.killed = true;
        crate::broker::command::kill_tree(&mut self.child).await;
    }

    /// Close stdin so a well-behaved worker exits, wait up to `grace`, then kill.
    pub async fn shutdown(mut self, grace: Duration) -> ExitReport {
        self.stdin = None;
        if tokio::time::timeout(grace, self.child.wait())
            .await
            .is_err()
        {
            self.kill().await;
        }
        self.finish().await
    }

    async fn exit_report_error(&mut self) -> WorkerError {
        self.stdin = None;
        // Give a crashed child a moment to be reaped so the code is available.
        if tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            self.kill().await;
        }
        WorkerError::Exited(self.finish().await)
    }

    async fn finish(&mut self) -> ExitReport {
        let status = self.child.wait().await.ok();
        if let Some(mut task) = self.stderr_task.take()
            && tokio::time::timeout(Duration::from_millis(250), &mut task)
                .await
                .is_err()
        {
            task.abort();
        }
        let bytes = self.stderr.lock().expect("stderr buffer poisoned").clone();
        ExitReport {
            code: status.and_then(|s| s.code()),
            stderr: harness_core::redaction::redact(&String::from_utf8_lossy(&bytes)),
            killed_by_supervisor: self.killed,
        }
    }
}

/// Async twin of `harness_core::framing::read_frame`, with the same size limit enforced
/// incrementally.
pub async fn read_frame_async<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<Envelope>, FrameError> {
    let mut line = Vec::new();
    let limit = (MAX_FRAME_BYTES + 2) as u64;
    let n = (&mut *reader)
        .take(limit)
        .read_until(b'\n', &mut line)
        .await
        .map_err(FrameError::Io)?;
    if n == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return if line.len() > MAX_FRAME_BYTES {
            Err(FrameError::Oversize {
                limit: MAX_FRAME_BYTES,
            })
        } else {
            Err(FrameError::Truncated)
        };
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    decode_frame(&line).map(Some)
}
