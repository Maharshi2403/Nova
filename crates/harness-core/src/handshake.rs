//! `hello` / `hello_ack` version handshake. Rust sends `hello`, the worker answers with a
//! `hello_ack` whose `correlation_id` is the hello's `message_id`. No implicit downgrade.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::protocol::{Envelope, MessageKind, ProtocolVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    NotHello(MessageKind),
    NotHelloAck(MessageKind),
    /// The ack answered a different request (or none at all).
    CorrelationMismatch {
        expected: String,
        actual: Option<String>,
    },
    UnsupportedVersion(String),
    MissingField(&'static str),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotHello(k) => write!(f, "expected hello, got {k:?}"),
            Self::NotHelloAck(k) => write!(f, "expected hello_ack, got {k:?}"),
            Self::CorrelationMismatch { expected, actual } => {
                write!(
                    f,
                    "hello_ack correlation {actual:?} does not match {expected}"
                )
            }
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v:?}"),
            Self::MissingField(n) => write!(f, "hello_ack payload is missing {n}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// What the worker agreed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub selected_version: ProtocolVersion,
    pub worker_version: String,
}

pub fn build_hello(message_id: Uuid, timestamp: impl Into<String>) -> Envelope {
    let mut payload = Map::new();
    payload.insert("runtime".into(), json!("rust"));
    payload.insert(
        "supported_versions".into(),
        json!(ProtocolVersion::SUPPORTED),
    );
    Envelope {
        protocol_version: ProtocolVersion::V1_0,
        kind: MessageKind::Hello,
        message_id,
        correlation_id: None,
        run_id: None,
        task_id: None,
        attempt: None,
        timestamp: timestamp.into(),
        payload,
    }
}

/// Check a worker's reply to `hello`. Both envelopes must already be individually valid.
pub fn verify_ack(hello: &Envelope, ack: &Envelope) -> Result<Accepted, HandshakeError> {
    if hello.kind != MessageKind::Hello {
        return Err(HandshakeError::NotHello(hello.kind));
    }
    if ack.kind != MessageKind::HelloAck {
        return Err(HandshakeError::NotHelloAck(ack.kind));
    }
    let expected = hello.message_id.to_string();
    if ack.correlation_id.as_deref() != Some(expected.as_str()) {
        return Err(HandshakeError::CorrelationMismatch {
            expected,
            actual: ack.correlation_id.clone(),
        });
    }
    let selected = ack
        .payload
        .get("selected_version")
        .and_then(Value::as_str)
        .ok_or(HandshakeError::MissingField("selected_version"))?;
    if !ProtocolVersion::SUPPORTED.contains(&selected) {
        return Err(HandshakeError::UnsupportedVersion(selected.to_string()));
    }
    // The worker must also have chosen from what Rust offered.
    let offered = hello
        .payload
        .get("supported_versions")
        .and_then(Value::as_array)
        .is_some_and(|v| v.iter().any(|x| x.as_str() == Some(selected)));
    if !offered {
        return Err(HandshakeError::UnsupportedVersion(selected.to_string()));
    }
    let worker_version = ack
        .payload
        .get("worker_version")
        .and_then(Value::as_str)
        .ok_or(HandshakeError::MissingField("worker_version"))?
        .to_string();
    Ok(Accepted {
        selected_version: ProtocolVersion::V1_0,
        worker_version,
    })
}
