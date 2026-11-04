//! Pure task state-transition function (see CONTRACTS.md section 3).

use crate::domain::TaskState::{self, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// The source state is terminal; terminal states never change.
    TerminalState(TaskState),
    Illegal {
        from: TaskState,
        to: TaskState,
    },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TerminalState(s) => write!(f, "{s:?} is terminal and cannot change"),
            Self::Illegal { from, to } => write!(f, "transition {from:?} -> {to:?} is not allowed"),
        }
    }
}

impl std::error::Error for TransitionError {}

/// Every allowed `(from, to)` pair: normal path first, then exception paths.
pub const ALLOWED: &[(TaskState, TaskState)] = &[
    (Proposed, Pending),
    (Proposed, Cancelled),
    (Pending, Ready),
    (Pending, Blocked),
    (Pending, Cancelled),
    (Ready, Leased),
    (Ready, Blocked),
    (Ready, Cancelled),
    (Leased, Running),
    (Leased, RetryWait),
    (Leased, Blocked),
    (Leased, Cancelled),
    (Running, Verifying),
    (Running, RetryWait),
    (Running, Blocked),
    (Running, Failed),
    (Running, Cancelled),
    (Verifying, Reviewing),
    (Verifying, RetryWait),
    (Verifying, Blocked),
    (Verifying, Failed),
    (Reviewing, Integrating),
    (Reviewing, Blocked),
    (Reviewing, Failed),
    (Integrating, Completed),
    (Integrating, Blocked),
    (Integrating, RetryWait),
    (Integrating, Failed),
    (RetryWait, Ready),
    (RetryWait, Blocked),
    (RetryWait, Failed),
    (RetryWait, Cancelled),
    (Blocked, Ready),
    (Blocked, Failed),
    (Blocked, Cancelled),
];

pub fn is_allowed(from: TaskState, to: TaskState) -> bool {
    ALLOWED.contains(&(from, to))
}

/// Validate a transition and return the new state.
pub fn transition(from: TaskState, to: TaskState) -> Result<TaskState, TransitionError> {
    if from.is_terminal() {
        return Err(TransitionError::TerminalState(from));
    }
    if is_allowed(from, to) {
        Ok(to)
    } else {
        Err(TransitionError::Illegal { from, to })
    }
}
