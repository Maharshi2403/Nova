//! Typed project, run and task domain. Pure data and validation; no I/O.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    Empty(&'static str),
    TooLong(&'static str, usize),
    PriorityOutOfRange(i64),
    InvalidLimit(&'static str),
    NoAcceptance,
    BadAcceptance(String),
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty(n) => write!(f, "{n} must not be empty"),
            Self::TooLong(n, max) => write!(f, "{n} exceeds {max} characters"),
            Self::PriorityOutOfRange(v) => write!(f, "priority {v} is outside 0..=100"),
            Self::InvalidLimit(n) => write!(f, "invalid limit: {n}"),
            Self::NoAcceptance => write!(f, "at least one acceptance check is required"),
            Self::BadAcceptance(m) => write!(f, "invalid acceptance check: {m}"),
        }
    }
}

impl std::error::Error for DomainError {}

fn bounded(name: &'static str, value: &str, max: usize) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::Empty(name));
    }
    if value.chars().count() > max {
        return Err(DomainError::TooLong(name, max));
    }
    Ok(())
}

macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                bounded(stringify!($name), &value, 128)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(
    /// Stable task identifier unique within a plan.
    TaskId
);
id_newtype!(RunId);
id_newtype!(ProjectId);

/// Task priority, validated to 0..=100 (higher runs sooner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Priority(u8);

impl Priority {
    pub const MAX: u8 = 100;

    pub fn new(value: i64) -> Result<Self, DomainError> {
        u8::try_from(value)
            .ok()
            .filter(|v| *v <= Self::MAX)
            .map(Self)
            .ok_or(DomainError::PriorityOutOfRange(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<i64> for Priority {
    type Error = DomainError;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Priority> for i64 {
    fn from(p: Priority) -> i64 {
        i64::from(p.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Proposed,
    Pending,
    Ready,
    Leased,
    Running,
    Verifying,
    Reviewing,
    Integrating,
    RetryWait,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Planning,
    AwaitingApproval,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Acceptance {
    /// Argument vector executed by Rust (never through a shell).
    Test {
        command: Vec<String>,
    },
    Assertion {
        description: String,
    },
}

impl Acceptance {
    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Test { command } => {
                if command.is_empty() || command.len() > 50 {
                    return Err(DomainError::BadAcceptance(
                        "command needs 1..=50 args".into(),
                    ));
                }
                if command[0].trim().is_empty() {
                    return Err(DomainError::BadAcceptance("empty program".into()));
                }
                if command.iter().any(|a| a.chars().count() > 1024) {
                    return Err(DomainError::BadAcceptance("argument too long".into()));
                }
                Ok(())
            }
            Self::Assertion { description } => bounded("assertion", description, 1000),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLimits {
    pub max_tool_calls: u32,
    pub max_output_tokens: u32,
    pub timeout_seconds: u32,
    pub cost_cap_usd: f64,
}

impl TaskLimits {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !(1..=1000).contains(&self.max_tool_calls) {
            return Err(DomainError::InvalidLimit("max_tool_calls"));
        }
        if !(1..=1_000_000).contains(&self.max_output_tokens) {
            return Err(DomainError::InvalidLimit("max_output_tokens"));
        }
        if !(1..=86_400).contains(&self.timeout_seconds) {
            return Err(DomainError::InvalidLimit("timeout_seconds"));
        }
        if !self.cost_cap_usd.is_finite() || self.cost_cap_usd < 0.0 {
            return Err(DomainError::InvalidLimit("cost_cap_usd"));
        }
        Ok(())
    }
}

/// Intended expertise; older plans default to coding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UseCase {
    #[default]
    Coding,
    Planning,
    Review,
    Debugging,
    Architecture,
    Fast,
}

/// Task as proposed by the planner (mirrors `schemas/task.schema.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    #[serde(default)]
    pub use_case: UseCase,
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub complexity: Complexity,
    pub risk: Risk,
    pub agent_priority: Priority,
    pub manual_priority: Option<Priority>,
    pub dependencies: Vec<TaskId>,
    pub candidate_paths: Vec<String>,
    pub acceptance: Vec<Acceptance>,
    pub limits: TaskLimits,
}

impl TaskSpec {
    pub fn validate(&self) -> Result<(), DomainError> {
        let title_len = self.title.trim().chars().count();
        if title_len == 0 {
            return Err(DomainError::Empty("title"));
        }
        if title_len < 3 || self.title.chars().count() > 200 {
            return Err(DomainError::TooLong("title", 200));
        }
        bounded("description", &self.description, 8000)?;
        if self.candidate_paths.len() > 100 {
            return Err(DomainError::TooLong("candidate_paths", 100));
        }
        for p in &self.candidate_paths {
            bounded("candidate_path", p, 1024)?;
        }
        if self.acceptance.is_empty() {
            return Err(DomainError::NoAcceptance);
        }
        self.acceptance.iter().try_for_each(Acceptance::validate)?;
        self.limits.validate()
    }
}

/// Persisted task: a validated spec plus runtime state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub run_id: RunId,
    pub spec: TaskSpec,
    pub state: TaskState,
    /// Increments on every mutable row update; used for compare-and-set.
    pub revision: u64,
    pub attempt: u32,
}

impl Task {
    /// Create a task in `proposed` state after validating the spec.
    pub fn new(run_id: RunId, spec: TaskSpec) -> Result<Self, DomainError> {
        spec.validate()?;
        Ok(Self {
            run_id,
            spec,
            state: TaskState::Proposed,
            revision: 0,
            attempt: 0,
        })
    }

    pub fn id(&self) -> &TaskId {
        &self.spec.id
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub repo_path: String,
}

impl Project {
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        repo_path: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let (name, repo_path) = (name.into(), repo_path.into());
        bounded("project name", &name, 200)?;
        bounded("repo_path", &repo_path, 4096)?;
        Ok(Self {
            id,
            name,
            repo_path,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub project_id: ProjectId,
    pub state: RunState,
    pub revision: u64,
}

impl Run {
    pub fn new(id: RunId, project_id: ProjectId) -> Self {
        Self {
            id,
            project_id,
            state: RunState::Planning,
            revision: 0,
        }
    }
}
