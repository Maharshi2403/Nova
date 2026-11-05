//! Task leases: exclusive, expiring ownership of one task attempt.

use harness_core::domain::{RunId, Task, TaskId, TaskState};
use harness_core::state;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::repository::{Repository, enum_to_text, to_i64};
use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub id: String,
    pub run_id: RunId,
    pub task_id: TaskId,
    pub worker_id: String,
    pub attempt: u32,
    pub granted_at: i64,
    pub expires_at: i64,
    pub released_at: Option<i64>,
}

/// Caller-chosen lease identity and clock; the id doubles as the event idempotency key.
#[derive(Debug, Clone)]
pub struct LeaseRequest<'a> {
    pub lease_id: &'a str,
    pub worker_id: &'a str,
    pub now: i64,
    pub ttl_seconds: u32,
}

/// A task moved out of a stale lease by [`LeaseRepository::reconcile_expired_leases`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    pub run_id: RunId,
    pub task_id: TaskId,
    pub lease_id: String,
    pub from: TaskState,
    pub to: TaskState,
}

#[allow(async_fn_in_trait)]
pub trait LeaseRepository {
    /// Atomically move a `ready` task to `leased` (revision compare-and-set), bump its
    /// attempt, insert the lease and append `task.leased`. At most one live lease per task.
    async fn grant_lease(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        expected_revision: u64,
        req: &LeaseRequest<'_>,
    ) -> Result<(Task, Lease), StoreError>;
    async fn active_lease(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Option<Lease>, StoreError>;
    /// Mark a lease released without touching the task (the caller transitions it).
    async fn release_lease(&self, lease_id: &str, now: i64) -> Result<(), StoreError>;
    /// Release every live lease with `expires_at <= now`. Tasks still in flight go to
    /// `retry_wait` (or `blocked` where that is the only legal exit) through a validated
    /// transition; they are never completed. Each recovery appends `lease.expired`.
    async fn reconcile_expired_leases(&self, now: i64) -> Result<Vec<Reconciled>, StoreError>;
}

fn row_to_lease(row: &SqliteRow) -> Result<Lease, StoreError> {
    let bad = |e: harness_core::domain::DomainError| StoreError::Corrupt(e.to_string());
    Ok(Lease {
        id: row.try_get("id")?,
        run_id: RunId::new(row.try_get::<String, _>("run_id")?).map_err(bad)?,
        task_id: TaskId::new(row.try_get::<String, _>("task_id")?).map_err(bad)?,
        worker_id: row.try_get("worker_id")?,
        attempt: u32::try_from(row.try_get::<i64, _>("attempt")?)
            .map_err(|_| StoreError::Corrupt("attempt out of range".into()))?,
        granted_at: row.try_get("granted_at")?,
        expires_at: row.try_get("expires_at")?,
        released_at: row.try_get("released_at")?,
    })
}

const LEASE_COLUMNS: &str =
    "id, run_id, task_id, worker_id, attempt, granted_at, expires_at, released_at";

impl LeaseRepository for Store {
    async fn grant_lease(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        expected_revision: u64,
        req: &LeaseRequest<'_>,
    ) -> Result<(Task, Lease), StoreError> {
        self.grant_lease_with_budget(run_id, task_id, expected_revision, req, None)
            .await
    }

    async fn active_lease(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
    ) -> Result<Option<Lease>, StoreError> {
        let row = sqlx::query(&format!(
            "SELECT {LEASE_COLUMNS} FROM leases \
             WHERE run_id = ? AND task_id = ? AND released_at IS NULL"
        ))
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .fetch_optional(self.pool())
        .await?;
        row.as_ref().map(row_to_lease).transpose()
    }

    async fn release_lease(&self, lease_id: &str, now: i64) -> Result<(), StoreError> {
        let result =
            sqlx::query("UPDATE leases SET released_at = ? WHERE id = ? AND released_at IS NULL")
                .bind(now)
                .bind(lease_id)
                .execute(self.pool())
                .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound("active lease"));
        }
        Ok(())
    }

    async fn reconcile_expired_leases(&self, now: i64) -> Result<Vec<Reconciled>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT {LEASE_COLUMNS} FROM leases WHERE released_at IS NULL AND expires_at <= ? \
             ORDER BY expires_at ASC, id ASC"
        ))
        .bind(now)
        .fetch_all(self.pool())
        .await?;

        let mut out = Vec::new();
        for row in &rows {
            let lease = row_to_lease(row)?;
            let task = self.get_task(&lease.run_id, &lease.task_id).await?;
            let to = if state::is_allowed(task.state, TaskState::RetryWait) {
                TaskState::RetryWait
            } else {
                TaskState::Blocked
            };
            let moves = !task.state.is_terminal() && state::is_allowed(task.state, to);

            let mut tx = self.pool().begin().await?;
            let released = sqlx::query(
                "UPDATE leases SET released_at = ? WHERE id = ? AND released_at IS NULL",
            )
            .bind(now)
            .bind(&lease.id)
            .execute(&mut *tx)
            .await?;
            if released.rows_affected() == 0 {
                // Someone else released it first.
                tx.rollback().await?;
                continue;
            }
            if moves {
                let updated = sqlx::query(
                    "UPDATE tasks SET state = ?, revision = revision + 1 \
                     WHERE run_id = ? AND id = ? AND revision = ?",
                )
                .bind(enum_to_text(&to)?)
                .bind(lease.run_id.as_str())
                .bind(lease.task_id.as_str())
                .bind(to_i64(task.revision)?)
                .execute(&mut *tx)
                .await?;
                if updated.rows_affected() == 0 {
                    // The task moved concurrently; leave the lease for the next pass.
                    tx.rollback().await?;
                    continue;
                }
            }
            sqlx::query(
                "INSERT INTO events (run_id, task_id, kind, from_state, to_state, attempt, \
                 idempotency_key) VALUES (?, ?, 'lease.expired', ?, ?, ?, ?)",
            )
            .bind(lease.run_id.as_str())
            .bind(lease.task_id.as_str())
            .bind(enum_to_text(&task.state)?)
            .bind(enum_to_text(&if moves { to } else { task.state })?)
            .bind(i64::from(lease.attempt))
            .bind(format!("lease-expired:{}", lease.id))
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            out.push(Reconciled {
                run_id: lease.run_id,
                task_id: lease.task_id,
                lease_id: lease.id,
                from: task.state,
                to: if moves { to } else { task.state },
            });
        }
        Ok(out)
    }
}

impl Store {
    pub(crate) async fn grant_lease_with_budget(
        &self,
        run_id: &RunId,
        task_id: &TaskId,
        expected_revision: u64,
        req: &LeaseRequest<'_>,
        budget: Option<crate::budgets::BudgetRequest>,
    ) -> Result<(Task, Lease), StoreError> {
        let current = self.get_task(run_id, task_id).await?;
        if current.revision != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        state::transition(current.state, TaskState::Leased)?;
        let attempt = i64::from(current.attempt) + 1;
        let expires_at = req.now + i64::from(req.ttl_seconds);

        let mut tx = self.pool().begin().await?;
        let result = sqlx::query(
            "UPDATE tasks SET state = 'leased', attempt = ?, revision = revision + 1 \
             WHERE run_id = ? AND id = ? AND revision = ? AND state = 'ready'",
        )
        .bind(attempt)
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .bind(to_i64(expected_revision)?)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            tx.rollback().await?;
            let actual = self.get_task(run_id, task_id).await?.revision;
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let inserted = sqlx::query(
            "INSERT INTO leases (id, run_id, task_id, worker_id, attempt, granted_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(req.lease_id)
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .bind(req.worker_id)
        .bind(attempt)
        .bind(req.now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await;
        if let Err(e) = inserted {
            tx.rollback().await?;
            return Err(StoreError::from_insert(e));
        }
        let event = sqlx::query(
            "INSERT INTO events (run_id, task_id, kind, from_state, to_state, attempt, \
             idempotency_key) VALUES (?, ?, 'task.leased', 'ready', 'leased', ?, ?)",
        )
        .bind(run_id.as_str())
        .bind(task_id.as_str())
        .bind(attempt)
        .bind(req.lease_id)
        .execute(&mut *tx)
        .await;
        if let Err(e) = event {
            tx.rollback().await?;
            return Err(StoreError::from_insert(e));
        }
        if let Some(budget) = budget {
            // The task UPDATE above already holds SQLite's writer lock, so concurrent
            // reservations cannot both observe the same remaining balance.
            let estimate = i64::try_from(budget.estimated_microdollars)
                .map_err(|_| StoreError::BudgetDenied("estimate overflow"))?;
            let limits = &current.spec.limits;
            let task_cap = limits.cost_cap_usd * 1_000_000.0;
            if !task_cap.is_finite()
                || task_cap < 0.0
                || estimate as f64 > task_cap.floor()
                || limits.max_output_tokens == 0
                || limits.max_tool_calls == 0
            {
                return Err(StoreError::BudgetDenied("task limits"));
            }
            let day = req.now.div_euclid(86_400);
            for (scope, cap, query, key) in [
                (
                    "task",
                    task_cap.min(i64::MAX as f64) as u64,
                    "SELECT COALESCE(SUM(amount), 0) FROM budget_reservations WHERE released = 0 AND run_id = ? AND task_id = ?",
                    task_id.as_str().to_string(),
                ),
                (
                    "run",
                    budget.run_cap_microdollars,
                    "SELECT COALESCE(SUM(amount), 0) FROM budget_reservations WHERE released = 0 AND run_id = ?",
                    String::new(),
                ),
                (
                    "daily",
                    budget.daily_cap_microdollars,
                    "SELECT COALESCE(SUM(amount), 0) FROM budget_reservations WHERE released = 0 AND utc_day = ?",
                    String::new(),
                ),
            ] {
                let used: i64 = if scope == "daily" {
                    sqlx::query_scalar(query)
                        .bind(day)
                        .fetch_one(&mut *tx)
                        .await?
                } else if scope == "task" {
                    sqlx::query_scalar(query)
                        .bind(run_id.as_str())
                        .bind(key)
                        .fetch_one(&mut *tx)
                        .await?
                } else {
                    sqlx::query_scalar(query)
                        .bind(run_id.as_str())
                        .fetch_one(&mut *tx)
                        .await?
                };
                if (used as u64)
                    .checked_add(budget.estimated_microdollars)
                    .is_none_or(|total| total > cap)
                {
                    return Err(StoreError::BudgetDenied(scope));
                }
            }
            sqlx::query("INSERT INTO budget_reservations (lease_id, run_id, task_id, utc_day, amount) VALUES (?, ?, ?, ?, ?)")
                .bind(req.lease_id).bind(run_id.as_str()).bind(task_id.as_str()).bind(day).bind(estimate)
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;

        let task = self.get_task(run_id, task_id).await?;
        let lease = self
            .active_lease(run_id, task_id)
            .await?
            .ok_or_else(|| StoreError::Corrupt("lease missing after grant".into()))?;
        Ok((task, lease))
    }
}
