//! Immutable plan versions with an explicit approval step.
//!
//! A proposed plan is validated in Rust, stored verbatim, and its tasks are created in
//! `proposed` state. Nothing can be dispatched until `approve_plan` moves the run to
//! `running` and the tasks to `pending`.

use harness_core::domain::{RunId, RunState, TaskId, TaskSpec, TaskState};
use harness_core::plan::PlanProposal;
use harness_core::state;
use sqlx::Row;

use crate::repository::{enum_to_text, to_i64};
use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStatus {
    Proposed,
    Approved,
    Rejected,
}

impl PlanStatus {
    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "proposed" => Ok(Self::Proposed),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            other => Err(StoreError::Corrupt(format!("plan status {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanRecord {
    pub run_id: RunId,
    pub version: u32,
    pub status: PlanStatus,
    pub plan: PlanProposal,
}

#[allow(async_fn_in_trait)]
pub trait PlanRepository {
    /// Validate and persist a new immutable plan version for a run in `planning`, create
    /// its tasks (`proposed`) and move the run to `awaiting_approval`. An invalid plan
    /// changes nothing.
    async fn propose_plan(
        &self,
        run_id: &RunId,
        plan: &PlanProposal,
    ) -> Result<PlanRecord, StoreError>;
    /// Approve the proposed plan: run -> `running`, tasks `proposed` -> `pending`.
    async fn approve_plan(&self, run_id: &RunId, version: u32) -> Result<PlanRecord, StoreError>;
    async fn get_plan(&self, run_id: &RunId, version: u32) -> Result<PlanRecord, StoreError>;
}

impl Store {
    async fn load_plan(&self, run_id: &RunId, version: u32) -> Result<PlanRecord, StoreError> {
        let row =
            sqlx::query("SELECT status, content_json FROM plans WHERE run_id = ? AND version = ?")
                .bind(run_id.as_str())
                .bind(i64::from(version))
                .fetch_optional(self.pool())
                .await?
                .ok_or(StoreError::NotFound("plan"))?;
        let content: String = row.try_get("content_json")?;
        Ok(PlanRecord {
            run_id: run_id.clone(),
            version,
            status: PlanStatus::parse(row.try_get::<&str, _>("status")?)?,
            plan: serde_json::from_str(&content)?,
        })
    }

    async fn set_run_state(
        tx: &mut sqlx::SqliteConnection,
        run_id: &RunId,
        from: RunState,
        to: RunState,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "UPDATE runs SET state = ?, revision = revision + 1 WHERE id = ? AND state = ?",
        )
        .bind(enum_to_text(&to)?)
        .bind(run_id.as_str())
        .bind(enum_to_text(&from)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::PlanNotDecidable(
                "run state changed concurrently",
            ));
        }
        sqlx::query(
            "INSERT INTO events (run_id, kind, from_state, to_state) VALUES (?, 'run.transition', ?, ?)",
        )
        .bind(run_id.as_str())
        .bind(enum_to_text(&from)?)
        .bind(enum_to_text(&to)?)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }
}

impl PlanRepository for Store {
    async fn propose_plan(
        &self,
        run_id: &RunId,
        plan: &PlanProposal,
    ) -> Result<PlanRecord, StoreError> {
        self.propose_plan_inner(run_id, plan, None).await
    }

    async fn approve_plan(&self, run_id: &RunId, version: u32) -> Result<PlanRecord, StoreError> {
        let record = self.load_plan(run_id, version).await?;
        if record.status != PlanStatus::Proposed {
            return Err(StoreError::PlanNotDecidable("plan already decided"));
        }
        state::transition(TaskState::Proposed, TaskState::Pending)?;
        let mut tx = self.pool().begin().await?;
        Self::set_run_state(
            &mut tx,
            run_id,
            RunState::AwaitingApproval,
            RunState::Running,
        )
        .await?;
        sqlx::query(
            "UPDATE plans SET status = 'approved', decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE run_id = ? AND version = ? AND status = 'proposed'",
        )
        .bind(run_id.as_str())
        .bind(i64::from(version))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE tasks SET state = 'pending', revision = revision + 1 \
             WHERE run_id = ? AND state = 'proposed'",
        )
        .bind(run_id.as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO events (run_id, kind, attempt) VALUES (?, 'plan.approved', ?)")
            .bind(run_id.as_str())
            .bind(to_i64(u64::from(version))?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.load_plan(run_id, version).await
    }

    async fn get_plan(&self, run_id: &RunId, version: u32) -> Result<PlanRecord, StoreError> {
        self.load_plan(run_id, version).await
    }
}

impl Store {
    /// Persist the generated plan and its planning evidence in one transaction.
    pub async fn propose_planned_goal(
        &self,
        run_id: &RunId,
        plan: &PlanProposal,
        detail: &serde_json::Value,
    ) -> Result<PlanRecord, StoreError> {
        self.propose_plan_inner(run_id, plan, Some(detail)).await
    }

    /// Startup-only: no planner from the previous daemon instance can still own a run.
    pub async fn recover_interrupted_planning(&self) -> Result<usize, StoreError> {
        let mut tx = self.pool().begin().await?;
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM runs WHERE state = 'planning'")
            .fetch_all(&mut *tx)
            .await?;
        for id in &ids {
            let run = RunId::new(id.clone()).map_err(|_| StoreError::Corrupt("run id".into()))?;
            Self::set_run_state(&mut tx, &run, RunState::Planning, RunState::Failed).await?;
            sqlx::query(
                "INSERT INTO events (run_id, kind, payload_json) VALUES (?, 'planning.failed', ?)",
            )
            .bind(id)
            .bind(serde_json::json!({"reason": "planning interrupted"}).to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(ids.len())
    }
    async fn propose_plan_inner(
        &self,
        run_id: &RunId,
        plan: &PlanProposal,
        detail: Option<&serde_json::Value>,
    ) -> Result<PlanRecord, StoreError> {
        let order = plan.validate()?;
        let content = serde_json::to_string(plan)?;
        let by_id: std::collections::HashMap<&TaskId, &TaskSpec> =
            plan.tasks.iter().map(|t| (&t.id, t)).collect();

        let mut tx = self.pool().begin().await?;
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM runs WHERE id = ?")
            .bind(run_id.as_str())
            .fetch_optional(&mut *tx)
            .await?;
        if state.as_deref() != Some("planning") {
            return Err(StoreError::PlanNotDecidable("run is not in planning"));
        }

        let version: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM plans WHERE run_id = ?")
                .bind(run_id.as_str())
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query(
            "INSERT INTO plans (run_id, version, status, content_json) VALUES (?, ?, 'proposed', ?)",
        )
        .bind(run_id.as_str())
        .bind(version)
        .bind(&content)
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from_insert)?;

        // Dependency-first order satisfies the dependency foreign keys.
        for id in &order {
            let spec = by_id[id];
            sqlx::query(
                "INSERT INTO tasks (run_id, id, state, revision, attempt, agent_priority, \
                 manual_priority, spec_json) VALUES (?, ?, 'proposed', 0, 0, ?, ?, ?)",
            )
            .bind(run_id.as_str())
            .bind(id.as_str())
            .bind(i64::from(spec.agent_priority))
            .bind(spec.manual_priority.map(i64::from))
            .bind(serde_json::to_string(spec)?)
            .execute(&mut *tx)
            .await
            .map_err(StoreError::from_insert)?;
            for dep in &spec.dependencies {
                sqlx::query(
                    "INSERT OR IGNORE INTO task_dependencies (run_id, task_id, depends_on_id) \
                     VALUES (?, ?, ?)",
                )
                .bind(run_id.as_str())
                .bind(id.as_str())
                .bind(dep.as_str())
                .execute(&mut *tx)
                .await?;
            }
        }
        Self::set_run_state(
            &mut tx,
            run_id,
            RunState::Planning,
            RunState::AwaitingApproval,
        )
        .await?;
        sqlx::query("INSERT INTO events (run_id, kind, attempt) VALUES (?, 'plan.proposed', ?)")
            .bind(run_id.as_str())
            .bind(version)
            .execute(&mut *tx)
            .await?;
        if let Some(detail) = detail {
            sqlx::query("INSERT INTO events (run_id, kind, payload_json) VALUES (?, 'planning.completed', ?)")
                .bind(run_id.as_str()).bind(detail.to_string()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.load_plan(
            run_id,
            u32::try_from(version).map_err(|_| StoreError::Corrupt("version".into()))?,
        )
        .await
    }
}
