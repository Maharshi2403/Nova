//! Single-use approvals bound to exact argv, cwd and task attempt; every decision audited.
use crate::{Store, StoreError};
use harness_core::{
    approval::Action,
    domain::{RunId, TaskId},
};
use serde::Serialize;
use sqlx::Row;
#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub id: String,
    pub run_id: String,
    pub task_id: String,
    pub attempt: u32,
    pub action: Action,
    pub state: String,
    pub revision: i64,
}
impl Store {
    pub async fn list_approvals(&self, run: &RunId) -> Result<Vec<Approval>, StoreError> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM approvals WHERE run_id=? ORDER BY created_at,id LIMIT 1000",
        )
        .bind(run.as_str())
        .fetch_all(self.pool())
        .await?;
        let mut records = Vec::new();
        for id in ids {
            records.push(self.get_approval(&id).await?);
        }
        Ok(records)
    }
    pub async fn request_approval(
        &self,
        id: &str,
        run: &RunId,
        task: &TaskId,
        attempt: u32,
        action: &Action,
    ) -> Result<Approval, StoreError> {
        if id.is_empty()
            || action.argv.is_empty()
            || action.argv.len() > 64
            || action.argv.iter().any(|s| {
                s.len() > 8192 || s.contains('\0') || harness_core::redaction::redact(s) != *s
            })
        {
            return Err(StoreError::Corrupt(
                "invalid approval or credential material in argv".into(),
            ));
        }
        let json = serde_json::to_string(action)?;
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query("SELECT attempt,state FROM tasks WHERE run_id=? AND id=?")
            .bind(run.as_str())
            .bind(task.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let current: i64 = row.try_get("attempt")?;
        let state: String = row.try_get("state")?;
        if current != i64::from(attempt)
            || matches!(state.as_str(), "completed" | "failed" | "cancelled")
        {
            return Err(StoreError::PlanNotDecidable("stale approval attempt"));
        }
        let inserted = sqlx::query("INSERT INTO approvals(id,run_id,task_id,attempt,action_json) VALUES(?,?,?,?,?) ON CONFLICT(run_id,task_id,attempt,action_json) DO NOTHING").bind(id).bind(run.as_str()).bind(task.as_str()).bind(i64::from(attempt)).bind(&json).execute(&mut *tx).await?;
        if inserted.rows_affected() > 0 {
            sqlx::query("INSERT INTO events(run_id,task_id,kind,idempotency_key,payload_json) VALUES(?,?,'approval.requested',?,?)").bind(run.as_str()).bind(task.as_str()).bind(format!("approval:request:{id}")).bind(serde_json::json!({"approval_id":id,"operation":action.operation}).to_string()).execute(&mut *tx).await?;
        }
        let key: String = sqlx::query_scalar(
            "SELECT id FROM approvals WHERE run_id=? AND task_id=? AND attempt=? AND action_json=?",
        )
        .bind(run.as_str())
        .bind(task.as_str())
        .bind(i64::from(attempt))
        .bind(json)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_approval(&key).await
    }
    pub async fn get_approval(&self, id: &str) -> Result<Approval, StoreError> {
        let row = sqlx::query("SELECT * FROM approvals WHERE id=?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or(StoreError::NotFound("approval"))?;
        Ok(Approval {
            id: row.try_get("id")?,
            run_id: row.try_get("run_id")?,
            task_id: row.try_get("task_id")?,
            attempt: u32::try_from(row.try_get::<i64, _>("attempt")?)
                .map_err(|_| StoreError::Corrupt("approval attempt".into()))?,
            action: serde_json::from_str(row.try_get("action_json")?)?,
            state: row.try_get("state")?,
            revision: row.try_get("revision")?,
        })
    }
    pub async fn decide_approval(
        &self,
        id: &str,
        revision: i64,
        approved: bool,
    ) -> Result<Approval, StoreError> {
        let a = self.get_approval(id).await?;
        let state = if approved { "approved" } else { "denied" };
        let mut tx = self.pool().begin().await?;
        let changed = sqlx::query("UPDATE approvals SET state=?,revision=revision+1,decided_by='local_user' WHERE id=? AND revision=? AND state='pending' AND EXISTS(SELECT 1 FROM tasks t WHERE t.run_id=approvals.run_id AND t.id=approvals.task_id AND t.attempt=approvals.attempt AND t.state NOT IN ('completed','failed','cancelled'))").bind(state).bind(id).bind(revision).execute(&mut *tx).await?;
        if changed.rows_affected() == 0 {
            return Err(StoreError::PlanNotDecidable(
                "stale or already decided approval",
            ));
        }
        sqlx::query("INSERT INTO events(run_id,task_id,kind,idempotency_key,payload_json) VALUES(?,?,'approval.decided',?,?)").bind(&a.run_id).bind(&a.task_id).bind(format!("approval:decision:{id}")).bind(serde_json::json!({"approval_id":id,"decision":state,"actor":"local_user"}).to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        self.get_approval(id).await
    }
    pub async fn consume_approval(&self, id: &str, action: &Action) -> Result<bool, StoreError> {
        let a = self.get_approval(id).await?;
        if &a.action != action {
            return Ok(false);
        }
        let mut tx = self.pool().begin().await?;
        let changed = sqlx::query("UPDATE approvals SET state='consumed',revision=revision+1 WHERE id=? AND state='approved' AND EXISTS(SELECT 1 FROM tasks t WHERE t.run_id=approvals.run_id AND t.id=approvals.task_id AND t.attempt=approvals.attempt AND t.state NOT IN ('completed','failed','cancelled'))").bind(id).execute(&mut *tx).await?;
        if changed.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("INSERT INTO events(run_id,task_id,kind,idempotency_key,payload_json) VALUES(?,?,'approval.consumed',?,?)").bind(a.run_id).bind(a.task_id).bind(format!("approval:consume:{id}")).bind(serde_json::json!({"approval_id":id}).to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}
