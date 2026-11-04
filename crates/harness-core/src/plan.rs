//! Plan proposal contract (mirrors `schemas/plan.schema.json`) and its validation.

use serde::{Deserialize, Serialize};

use crate::dag::{DagError, topological_order};
use crate::domain::{DomainError, TaskId, TaskSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    pub recommended: String,
    pub alternatives: Vec<String>,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanProposal {
    pub schema_version: String,
    pub project_id: String,
    pub idea: String,
    pub assumptions: Vec<String>,
    pub requirements: Vec<String>,
    pub architecture: Architecture,
    pub tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    Malformed(String),
    Invalid(String),
    Task { id: TaskId, error: DomainError },
    Dag(DagError),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(e) => write!(f, "malformed plan: {e}"),
            Self::Invalid(e) => write!(f, "invalid plan: {e}"),
            Self::Task { id, error } => write!(f, "task {id}: {error}"),
            Self::Dag(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<DagError> for PlanError {
    fn from(e: DagError) -> Self {
        Self::Dag(e)
    }
}

impl PlanProposal {
    pub fn from_json(text: &str) -> Result<Self, PlanError> {
        serde_json::from_str(text).map_err(|e| PlanError::Malformed(e.to_string()))
    }

    /// Validate every field and the task graph. Returns task ids in a stable
    /// dependency-first order, which is the order they must be inserted.
    pub fn validate(&self) -> Result<Vec<TaskId>, PlanError> {
        if self.schema_version != "1.0" {
            return Err(PlanError::Invalid(format!(
                "unsupported schema_version {:?}",
                self.schema_version
            )));
        }
        if self.project_id.trim().is_empty() {
            return Err(PlanError::Invalid("project_id must not be empty".into()));
        }
        let idea = self.idea.chars().count();
        if !(5..=16_000).contains(&idea) {
            return Err(PlanError::Invalid(
                "idea must be 5..=16000 characters".into(),
            ));
        }
        if self.assumptions.len() > 100 {
            return Err(PlanError::Invalid("at most 100 assumptions".into()));
        }
        if self.requirements.is_empty() || self.requirements.iter().any(|r| r.trim().is_empty()) {
            return Err(PlanError::Invalid(
                "requirements must be non-empty strings".into(),
            ));
        }
        if self.architecture.recommended.chars().count() < 10 {
            return Err(PlanError::Invalid(
                "architecture.recommended must be at least 10 characters".into(),
            ));
        }
        if self.tasks.is_empty() || self.tasks.len() > 1000 {
            return Err(PlanError::Invalid("a plan needs 1..=1000 tasks".into()));
        }
        for t in &self.tasks {
            t.validate().map_err(|error| PlanError::Task {
                id: t.id.clone(),
                error,
            })?;
        }
        let nodes: Vec<(TaskId, Vec<TaskId>)> = self
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t.dependencies.clone()))
            .collect();
        Ok(topological_order(&nodes)?)
    }
}
