//! Durable pre-dispatch reservations. Amounts are integer USD microdollars.
//! Successful attempts retain their reservation until usage settlement is available.
use crate::leases::{Lease, LeaseRequest};
use crate::{Store, StoreError};
use harness_core::domain::{RunId, Task, TaskId};

#[derive(Debug, Clone, Copy)]
pub struct BudgetRequest {
    pub estimated_microdollars: u64,
    pub run_cap_microdollars: u64,
    /// Global UTC calendar day cap across runs.
    pub daily_cap_microdollars: u64,
}

impl Store {
    /// Reserve spend and grant a lease in the same transaction, before spawning a worker.
    pub async fn grant_budgeted_lease(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        revision: u64,
        lease: &LeaseRequest<'_>,
        budget: BudgetRequest,
    ) -> Result<(Task, Lease), StoreError> {
        self.grant_lease_with_budget(run_id, task_id, revision, lease, Some(budget))
            .await
    }

    /// Reconcile a spawn failure before any worker code ran. Release the lease and
    /// reservation, move the task to retry_wait, and append evidence atomically.
    /// Retries of this reconciliation are harmless.
    pub async fn release_failed_dispatch_budget(&self, lease_id: &str) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        // Acquire the writer lock before reading the current attempt state.
        sqlx::query("UPDATE budget_reservations SET released = released WHERE lease_id = ?")
            .bind(lease_id)
            .execute(&mut *tx)
            .await?;
        let row: Option<(String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT b.run_id, b.task_id, b.released, l.granted_at, l.attempt FROM budget_reservations b JOIN leases l ON l.id = b.lease_id WHERE b.lease_id = ?")
            .bind(lease_id).fetch_optional(&mut *tx).await?;
        let (run, task, released, now, attempt) =
            row.ok_or(StoreError::NotFound("budget reservation"))?;
        if released == 1 {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query("UPDATE tasks SET state = 'retry_wait', revision = revision + 1 WHERE run_id = ? AND id = ? AND state = 'leased' AND attempt = ?")
            .bind(&run).bind(&task).bind(attempt).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::BudgetDenied("attempt already started"));
        }
        sqlx::query("UPDATE leases SET released_at = ? WHERE id = ? AND released_at IS NULL")
            .bind(now)
            .bind(lease_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE budget_reservations SET released = 1 WHERE lease_id = ?")
            .bind(lease_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events (run_id, task_id, kind, from_state, to_state, attempt, idempotency_key) VALUES (?, ?, 'dispatch.failed', 'leased', 'retry_wait', ?, ?)")
            .bind(run).bind(task).bind(attempt).bind(format!("dispatch-failed:{lease_id}"))
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
