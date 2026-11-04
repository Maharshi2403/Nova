//! Typed task-assignment messages and per-attempt correlation enforcement.
//!
//! [`AssignmentSession`] is the Rust-side referee for one worker attempt: it checks scope,
//! rejects duplicate ids, allows one outstanding tool request, matches responses to the
//! exact request id and accepts exactly one terminal message.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::protocol::{Envelope, MessageKind, ProtocolVersion};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartTask {
    pub role: String,
    pub worktree_id: String,
    pub model: ModelRef,
    pub context_manifest_hash: String,
    pub allowed_tools: Vec<String>,
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
    pub timeout_seconds: u32,
    pub cost_cap_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRequest {
    pub tool: String,
    pub arguments: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    RegistryEstimate,
    ProviderBilled,
    Unknown,
}

/// Unknown counts are `None`, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub provider: String,
    pub model_id: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub price_source: PriceSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completed {
    pub summary: String,
    pub changed_paths: Vec<String>,
    pub claimed_tests: Vec<String>,
    #[serde(default)]
    pub subtask_requests: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failed {
    pub reason: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerMessage {
    Progress(Progress),
    ToolRequest {
        request_id: Uuid,
        request: ToolRequest,
    },
    Usage(Usage),
    Completed(Completed),
    Failed(Failed),
    Heartbeat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    BadPayload(MessageKind, String),
    WrongScope,
    Duplicate(Uuid),
    UnexpectedKind(MessageKind),
    AfterTerminal,
    ToolRequestOutstanding,
    NoToolRequestOutstanding,
    UnknownCorrelation(String),
    ToolCallLimit(u32),
    CompletedWithOpenRequest,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPayload(k, e) => write!(f, "invalid {k:?} payload: {e}"),
            Self::WrongScope => write!(f, "message does not belong to this assignment"),
            Self::Duplicate(id) => write!(f, "duplicate message_id {id}"),
            Self::UnexpectedKind(k) => write!(f, "{k:?} is not allowed here"),
            Self::AfterTerminal => write!(f, "message after the attempt finished"),
            Self::ToolRequestOutstanding => write!(f, "a tool request is already outstanding"),
            Self::NoToolRequestOutstanding => write!(f, "no tool request is outstanding"),
            Self::UnknownCorrelation(c) => write!(f, "unknown correlation_id {c}"),
            Self::ToolCallLimit(n) => write!(f, "tool call limit of {n} exceeded"),
            Self::CompletedWithOpenRequest => {
                write!(f, "terminal message while a tool request is outstanding")
            }
        }
    }
}

impl std::error::Error for SessionError {}

fn payload<T: serde::de::DeserializeOwned>(env: &Envelope) -> Result<T, SessionError> {
    serde_json::from_value(Value::Object(env.payload.clone()))
        .map_err(|e| SessionError::BadPayload(env.kind, e.to_string()))
}

fn to_map<T: Serialize>(value: &T) -> Map<String, Value> {
    match serde_json::to_value(value).expect("message payloads always serialize") {
        Value::Object(m) => m,
        _ => unreachable!("message payloads are JSON objects"),
    }
}

#[derive(Debug)]
pub struct AssignmentSession {
    run_id: String,
    task_id: String,
    attempt: u32,
    max_tool_calls: u32,
    tool_calls: u32,
    seen: HashSet<Uuid>,
    pending: Option<Uuid>,
    terminal: bool,
}

impl AssignmentSession {
    pub fn new(run_id: &str, task_id: &str, attempt: u32, max_tool_calls: u32) -> Self {
        Self {
            run_id: run_id.to_owned(),
            task_id: task_id.to_owned(),
            attempt,
            max_tool_calls,
            tool_calls: 0,
            seen: HashSet::new(),
            pending: None,
            terminal: false,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.terminal
    }

    pub fn pending_request(&self) -> Option<Uuid> {
        self.pending
    }

    fn in_scope(&self, env: &Envelope) -> bool {
        env.run_id.as_deref() == Some(&self.run_id)
            && env.task_id.as_deref() == Some(&self.task_id)
            && env.attempt == Some(self.attempt)
    }

    fn envelope(
        &self,
        kind: MessageKind,
        correlation_id: Option<String>,
        timestamp: String,
        payload: Map<String, Value>,
    ) -> Envelope {
        Envelope {
            protocol_version: ProtocolVersion::V1_0,
            kind,
            message_id: Uuid::new_v4(),
            correlation_id,
            run_id: Some(self.run_id.clone()),
            task_id: Some(self.task_id.clone()),
            attempt: Some(self.attempt),
            timestamp,
            payload,
        }
    }

    /// Build the `start_task` assignment (Rust -> worker).
    pub fn start_task(&self, start: &StartTask, timestamp: String) -> Envelope {
        self.envelope(MessageKind::StartTask, None, timestamp, to_map(start))
    }

    /// Validate and classify one worker -> Rust message.
    pub fn accept(&mut self, env: &Envelope) -> Result<WorkerMessage, SessionError> {
        if self.terminal {
            return Err(SessionError::AfterTerminal);
        }
        if env.kind != MessageKind::Heartbeat && !self.in_scope(env) {
            return Err(SessionError::WrongScope);
        }
        if self.seen.contains(&env.message_id) {
            return Err(SessionError::Duplicate(env.message_id));
        }
        let message = match env.kind {
            MessageKind::Heartbeat => WorkerMessage::Heartbeat,
            MessageKind::Progress => WorkerMessage::Progress(payload(env)?),
            MessageKind::Usage => WorkerMessage::Usage(payload(env)?),
            MessageKind::ToolRequest => {
                let request: ToolRequest = payload(env)?;
                if request.tool.trim().is_empty() {
                    return Err(SessionError::BadPayload(env.kind, "empty tool".into()));
                }
                if self.pending.is_some() {
                    return Err(SessionError::ToolRequestOutstanding);
                }
                if self.tool_calls >= self.max_tool_calls {
                    return Err(SessionError::ToolCallLimit(self.max_tool_calls));
                }
                self.tool_calls += 1;
                self.pending = Some(env.message_id);
                WorkerMessage::ToolRequest {
                    request_id: env.message_id,
                    request,
                }
            }
            MessageKind::Completed => {
                let done: Completed = payload(env)?;
                if done.summary.trim().is_empty() {
                    return Err(SessionError::BadPayload(env.kind, "empty summary".into()));
                }
                if self.pending.is_some() {
                    return Err(SessionError::CompletedWithOpenRequest);
                }
                self.terminal = true;
                WorkerMessage::Completed(done)
            }
            MessageKind::Failed => {
                let failed: Failed = payload(env)?;
                if failed.reason.trim().is_empty() {
                    return Err(SessionError::BadPayload(env.kind, "empty reason".into()));
                }
                self.terminal = true;
                WorkerMessage::Failed(failed)
            }
            other => return Err(SessionError::UnexpectedKind(other)),
        };
        self.seen.insert(env.message_id);
        Ok(message)
    }

    /// Build the `tool_response` for the outstanding request and clear it.
    pub fn tool_response(
        &mut self,
        request_id: Uuid,
        response: &ToolResponse,
        timestamp: String,
    ) -> Result<Envelope, SessionError> {
        match self.pending {
            None => Err(SessionError::NoToolRequestOutstanding),
            Some(p) if p != request_id => {
                Err(SessionError::UnknownCorrelation(request_id.to_string()))
            }
            Some(_) => {
                let good = if response.ok {
                    response.result.is_some() && response.error.is_none()
                } else {
                    response.error.is_some() && response.result.is_none()
                };
                if !good {
                    return Err(SessionError::BadPayload(
                        MessageKind::ToolResponse,
                        "ok requires result; failure requires error".into(),
                    ));
                }
                self.pending = None;
                Ok(self.envelope(
                    MessageKind::ToolResponse,
                    Some(request_id.to_string()),
                    timestamp,
                    to_map(response),
                ))
            }
        }
    }
}
