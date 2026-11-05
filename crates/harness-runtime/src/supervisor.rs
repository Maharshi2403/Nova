//! Supervises one assignment on a handshaken worker: heartbeat/idle timeout, an overall
//! deadline, cooperative cancellation and a hard cap on worker messages.
//!
//! The worker is read one frame at a time, so a flooding worker is throttled by the OS pipe
//! instead of growing a queue; the message cap then ends the attempt.

use std::time::Duration;

use harness_core::messages::{
    AssignmentSession, Completed, Failed, SessionError, StartTask, ToolRequest, ToolResponse,
    WorkerMessage,
};
use harness_core::protocol::{Envelope, MessageKind, ProtocolVersion};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::watch;
use uuid::Uuid;

use crate::worker::{Worker, WorkerError};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Longest silence tolerated between any two worker frames (heartbeats count).
    pub idle_timeout: Duration,
    /// Wall-clock budget for the whole assignment.
    pub deadline: Duration,
    /// Frames accepted from the worker before the attempt is aborted.
    pub max_messages: u32,
}

#[derive(Debug)]
pub enum Outcome {
    Completed(Completed),
    Failed(Failed),
}

#[derive(Debug, thiserror::Error)]
pub enum SuperviseError {
    #[error("worker sent nothing for {0:?}")]
    IdleTimeout(Duration),
    #[error("assignment exceeded its {0:?} deadline")]
    DeadlineExceeded(Duration),
    #[error("worker sent more than {0} messages")]
    MessageFlood(u32),
    #[error("assignment budget exceeded: {0}")]
    BudgetExceeded(&'static str),
    #[error("assignment cancelled")]
    Cancelled,
    #[error("worker closed its stdout before finishing")]
    ClosedEarly,
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Worker(WorkerError),
}

/// Executes brokered tool requests on behalf of a worker.
#[allow(async_fn_in_trait)]
pub trait ToolBroker {
    async fn handle(&mut self, request: &ToolRequest) -> ToolResponse;
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formatting of the current time cannot fail")
}

fn cancel_envelope() -> Envelope {
    Envelope {
        protocol_version: ProtocolVersion::V1_0,
        kind: MessageKind::Cancel,
        message_id: Uuid::new_v4(),
        correlation_id: None,
        run_id: None,
        task_id: None,
        attempt: None,
        timestamp: now(),
        payload: serde_json::Map::new(),
    }
}

/// Drive the assignment to a terminal message. On any error the worker is killed (except
/// after `Cancelled`, where it has been asked to stop and the caller should `shutdown` it
/// with a grace period). The caller still owns the worker for its exit report.
pub async fn supervise<B: ToolBroker>(
    worker: &mut Worker,
    session: &mut AssignmentSession,
    start: &StartTask,
    limits: Limits,
    broker: &mut B,
    mut cancel: watch::Receiver<bool>,
) -> Result<Outcome, SuperviseError> {
    let result = tokio::time::timeout(
        limits.deadline,
        drive(worker, session, start, limits, broker, &mut cancel),
    )
    .await
    .unwrap_or(Err(SuperviseError::DeadlineExceeded(limits.deadline)));
    if !matches!(result, Ok(_) | Err(SuperviseError::Cancelled)) {
        worker.kill().await;
    }
    result
}

async fn drive<B: ToolBroker>(
    worker: &mut Worker,
    session: &mut AssignmentSession,
    start: &StartTask,
    limits: Limits,
    broker: &mut B,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Outcome, SuperviseError> {
    if start.max_output_tokens == 0
        || start.max_tool_calls == 0
        || !start.cost_cap_usd.is_finite()
        || start.cost_cap_usd < 0.0
    {
        return Err(SuperviseError::BudgetExceeded("invalid assignment limits"));
    }
    let mut tool_calls = 0u32;
    let mut output_tokens = 0u64;
    let mut cost = 0.0f64;
    let assignment = session.start_task(start, now());
    worker
        .send(&assignment)
        .await
        .map_err(SuperviseError::Worker)?;
    let mut received = 0u32;
    loop {
        if *cancel.borrow() {
            return cancelled(worker).await;
        }
        let frame = tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() {
                    return cancelled(worker).await;
                }
                continue;
            }
            frame = worker.recv(limits.idle_timeout) => frame,
        };
        let envelope = match frame {
            Ok(Some(e)) => e,
            Ok(None) => return Err(SuperviseError::ClosedEarly),
            Err(WorkerError::Timeout(d)) => return Err(SuperviseError::IdleTimeout(d)),
            Err(WorkerError::Frame(e)) => return Err(SuperviseError::Protocol(e.to_string())),
            Err(e) => return Err(SuperviseError::Worker(e)),
        };
        received += 1;
        if received > limits.max_messages {
            return Err(SuperviseError::MessageFlood(limits.max_messages));
        }
        match session.accept(&envelope)? {
            WorkerMessage::Progress(_) | WorkerMessage::Heartbeat => {}
            WorkerMessage::Usage(usage) => {
                output_tokens = output_tokens.saturating_add(usage.output_tokens.unwrap_or(0));
                if let Some(amount) = usage.cost_usd {
                    if !amount.is_finite() || amount < 0.0 {
                        return Err(SuperviseError::BudgetExceeded("invalid reported cost"));
                    }
                    cost += amount;
                }
                if output_tokens > u64::from(start.max_output_tokens) || cost > start.cost_cap_usd {
                    return Err(SuperviseError::BudgetExceeded("reported usage"));
                }
            }
            WorkerMessage::ToolRequest {
                request_id,
                request,
            } => {
                if tool_calls >= start.max_tool_calls {
                    return Err(SuperviseError::BudgetExceeded("tool calls"));
                }
                tool_calls += 1;
                let response = broker.handle(&request).await;
                let reply = session.tool_response(request_id, &response, now())?;
                worker.send(&reply).await.map_err(SuperviseError::Worker)?;
            }
            WorkerMessage::Completed(c) => return Ok(Outcome::Completed(c)),
            WorkerMessage::Failed(f) => return Ok(Outcome::Failed(f)),
        }
    }
}

async fn cancelled(worker: &mut Worker) -> Result<Outcome, SuperviseError> {
    // Best effort: a worker that is already gone just makes this send fail.
    let _ = worker.send(&cancel_envelope()).await;
    Err(SuperviseError::Cancelled)
}
