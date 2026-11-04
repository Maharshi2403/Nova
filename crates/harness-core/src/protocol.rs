//! Worker IPC envelope (protocol v1). Mirrors `schemas/worker-envelope.schema.json`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Only protocol versions listed here are accepted; anything else fails to deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolVersion {
    #[serde(rename = "1.0")]
    V1_0,
}

impl ProtocolVersion {
    pub const SUPPORTED: &'static [&'static str] = &["1.0"];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Hello,
    HelloAck,
    StartTask,
    Progress,
    ToolRequest,
    ToolResponse,
    Usage,
    SubtaskRequest,
    PlanProposal,
    ReviewRecommendation,
    Completed,
    Failed,
    Heartbeat,
    Cancel,
}

impl MessageKind {
    /// Kinds that must carry run_id, task_id and attempt.
    pub fn is_task_scoped(self) -> bool {
        matches!(
            self,
            Self::StartTask
                | Self::ToolRequest
                | Self::ToolResponse
                | Self::Usage
                | Self::Completed
                | Self::Failed
                | Self::Progress
                | Self::SubtaskRequest
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub protocol_version: ProtocolVersion,
    pub kind: MessageKind,
    pub message_id: Uuid,
    pub correlation_id: Option<String>,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub attempt: Option<u32>,
    pub timestamp: String,
    pub payload: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    Malformed(String),
    MissingScope(MessageKind),
    MissingCorrelation,
    EmptyField(&'static str),
    TooManyPayloadFields,
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(e) => write!(f, "malformed envelope: {e}"),
            Self::MissingScope(k) => write!(f, "{k:?} requires run_id, task_id and attempt"),
            Self::MissingCorrelation => write!(f, "tool_response requires correlation_id"),
            Self::EmptyField(n) => write!(f, "{n} must not be empty"),
            Self::TooManyPayloadFields => write!(f, "payload exceeds 100 properties"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl Envelope {
    /// Parse and validate one JSON document against the v1 contract invariants.
    pub fn from_json(text: &str) -> Result<Self, EnvelopeError> {
        let env: Self =
            serde_json::from_str(text).map_err(|e| EnvelopeError::Malformed(e.to_string()))?;
        env.validate()?;
        Ok(env)
    }

    pub fn validate(&self) -> Result<(), EnvelopeError> {
        for (name, value) in [
            ("correlation_id", &self.correlation_id),
            ("run_id", &self.run_id),
            ("task_id", &self.task_id),
        ] {
            if value.as_deref() == Some("") {
                return Err(EnvelopeError::EmptyField(name));
            }
        }
        if self.attempt == Some(0) {
            return Err(EnvelopeError::EmptyField("attempt"));
        }
        if self.payload.len() > 100 {
            return Err(EnvelopeError::TooManyPayloadFields);
        }
        if self.kind == MessageKind::ToolResponse && self.correlation_id.is_none() {
            return Err(EnvelopeError::MissingCorrelation);
        }
        if self.kind.is_task_scoped()
            && (self.run_id.is_none() || self.task_id.is_none() || self.attempt.is_none())
        {
            return Err(EnvelopeError::MissingScope(self.kind));
        }
        Ok(())
    }
}
